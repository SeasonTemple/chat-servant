# /// script
# requires-python = ">=3.11,<3.12"
# dependencies = [
#     "rapidocr==3.10.0",
#     "onnxruntime==1.31.0",
#     "pillow>=10",
#     "numpy<2",
#     "opencv-python-headless>=4.9",
# ]
# ///
"""把一次 OCR 的耗时拆成 预处理 / det / rec / 后处理 / 端到端，并把「冷启动」拆实。

```
target\\bench-venv\\Scripts\\python.exe scripts/ocrbench/latency_breakdown.py \\
    --model small --repeat 12
```

## 为什么要重拆「冷启动」

上一轮报出过「冷启动 602 ms、稳态中位 897 ms」——**冷比热快**，反常。
两件事叠在一起造成了它：

1. `RapidOCR.__init__` **不建模型**：`self.text_det = None`、
   `self.text_rec = None`，三个会话都是首次 `__call__` 里由
   `_load_det_model()` / `_load_cls_model()` / `_load_rec_model()` 惰性建的。
   ⇒ 「构造完成」不等于「会话已建」，而**构造之后的那一次推理仍然是冷的**，
   它把会话建立的钱一起付掉了。
2. `route_b_on_real_frame.py` 打完 3 次调用后 `times.sort()`，
   然后把 `times[0]` 印成 `cold`。**那是 3 次里的最小值，不是第 1 次。**
   于是报出去的是「最快的那一次」，必然低于中位数。

⇒ 本脚本把三段分开单列：**构造 / 会话建立 / 建立后首推**，
并把「进程内首推」的真值报出来。

## 分段计时怎么来的

`rapidocr` 3.10.0 **没有**对外的分阶段计时接口，
但 `RapidOCR.__call__` 的函数体只有六步，本脚本逐字照抄这六步并各自计时
（`preprocess_img` / `detect_and_crop` / `cls_and_rotate` / `recognize_txt` /
`build_final_output`）。**为什么照抄而不是 monkeypatch 内部类**：patch 十来个子类
才能凑齐同样的边界，可读性与「跑的到底是不是同一条路径」都不如照抄可靠。

三个 ONNX 会话的**纯推理**时间由 `OrtInferSession.__call__` 上的一个
计时壳给出（det / cls / rec 三个实例分别打标），于是
`det_total - det_infer = det 的预处理 + DB 后处理`，可以继续细分。

## 等价性自检为什么比对输出而不比对耗时

第一版用「逐段求和 vs 原生 e2e」的**墙钟**做自检，阈值 3%，结果直接失败：
逐段求和 669 ms、原生 1029 ms，差 35%。
查下来不是走偏了，是**这台机器的噪声就有 ±50%**——同一张图交替跑 12 次，
单次从 531 ms 到 1200 ms（见报告里的 `noise` 字段与后台负载）。
后台有动画壁纸 + Chrome + 微信常驻，`GetSystemTimes` 量到的系统级占用
在采样期只有 20% 上下，但周期性尖峰足以污染单次采样。

⇒ 自检改成**语义等价**：逐段路径与原生路径的 boxes / txts 必须逐项相同。
输出相同就说明走的是同一条代码路径，耗时差只能来自机器噪声。
墙钟偏差仍然**报出来**（不藏），但它是观测值、不是判据。

⇒ 另一处改动：逐段与原生**交替**采样（每轮顺序对调），
这样即使机器在缓慢漂移，两侧吃到的漂移也是均摊的。
"""

from __future__ import annotations

import argparse
import json
import statistics
import sys
import time
from pathlib import Path

# ⚠ Windows 上 Python 的 stdout 默认是传统代码页（GitHub 的 Windows runner
#   上就是 cp1252），一句中文 print 就 UnicodeEncodeError 带栈回溯崩掉。
#   本仓在 scripts/fetch_models.py 上踩过（CI run 39037571268），
#   而 ocrbench 这一族当时**一个都没加**——守卫的不变量只管
#   「gate.ps1 会 python 调起的脚本」，人工跑的台架在射程外。
#   不能指望调用方设 PYTHONIOENCODING，必须在脚本里自己改。
#   errors="replace"：真有编不出来的字符时**降级成 ? 而不是崩**——
#   台架的失败模式必须是「读数不符」，不是「打印失败 ⇒ 栈回溯」。
for _stream in (sys.stdout, sys.stderr):
    if hasattr(_stream, "reconfigure"):        # 3.7+ 都有
        _stream.reconfigure(encoding="utf-8", errors="replace")

ROOT = Path(__file__).resolve().parents[2]
OUT = ROOT / "target" / "probe" / "ocrbench"
sys.path.insert(0, str(Path(__file__).resolve().parent))

MODEL_DIRS = {
    "small": ROOT / "target" / "models" / "ppocrv6-small",
    "tiny": ROOT / "target" / "models" / "ppocrv6-tiny",
}
MODEL_FILES = {
    "small": ("PP-OCRv6_det_small.onnx", "PP-OCRv6_rec_small.onnx"),
    "tiny": ("PP-OCRv6_det_tiny.onnx", "PP-OCRv6_rec_tiny.onnx"),
}

# 真值中心 = 消息行的中心 y（1× 坐标），由 measure_real_frame_font.py 的
# 边缘密度量出并与 `xtask ocr` 打印的 y 逐条复核过。
# ⚠ 按帧不同，所以**逐帧写死**，不共用一套默认值。
FRAMES = {
    "wechat-chat": {
        "path": ROOT / "target" / "probe" / "wechat-chat.png",
        "centers": [185, 241, 292, 348, 404, 460, 516],
    },
    "gws-chat": {
        "path": ROOT / "target" / "probe" / "gws-chat.png",
        "centers": [16, 70, 124, 180, 236, 292, 346, 402, 458, 514],
    },
}

# ONNX 推理计时的打标表：会话实例 id -> 阶段名。
_INFER_TAG: dict[int, str] = {}
# 本次调用各阶段的累计毫秒。
_INFER_MS: dict[str, float] = {}


def install_infer_timer() -> None:
    """给 `OrtInferSession.__call__` 套一个计时壳。

    只改这一个方法：det / cls / rec 三个任务都走它
    （`get_engine(EngineType.ONNXRUNTIME)` 返回的就是这个类），
    所以这是**唯一的推理入口**，不会有漏网的路径。
    """
    from rapidocr.inference_engine.onnxruntime import OrtInferSession

    orig = OrtInferSession.__call__

    def timed(self, input_content):
        tag = _INFER_TAG.get(id(self), "unknown")
        t0 = time.perf_counter()
        out = orig(self, input_content)
        _INFER_MS[tag] = _INFER_MS.get(tag, 0.0) + (time.perf_counter() - t0) * 1000
        return out

    OrtInferSession.__call__ = timed


def p95(vals: list[float]) -> float:
    """最近秩 P95。样本少时它约等于最大值，所以**必须连 n 一起报**。"""
    if not vals:
        return float("nan")
    s = sorted(vals)
    k = max(1, -(-95 * len(s) // 100))  # ceil(0.95 n)
    return s[k - 1]


def stat(vals: list[float]) -> dict:
    return {
        "n": len(vals),
        "median_ms": round(statistics.median(vals), 1),
        "p95_ms": round(p95(vals), 1),
        "min_ms": round(min(vals), 1),
        "max_ms": round(max(vals), 1),
    }


class CpuBusy:
    """采样区间的系统级 CPU 占用，用来给耗时数字标噪声底。

    `GetSystemTimes` 给 idle / kernel / user 三个 100ns 计数器，
    占用 = 1 - Δidle/Δtotal（`kernel` 已含 idle，所以只减 idle）。
    Windows 专用——本仓的 OCR 台架只在 Windows 上跑。
    """

    def __init__(self) -> None:
        self._t = time.perf_counter()
        self._idle = self._idle_total()

    @staticmethod
    def _idle_total() -> tuple[int, int]:
        import ctypes

        class FILETIME(ctypes.Structure):
            _fields_ = [("dwLowDateTime", ctypes.c_uint32),
                        ("dwHighDateTime", ctypes.c_uint32)]

            def as_int(self) -> int:
                return (self.dwHighDateTime << 32) | self.dwLowDateTime

        k, u, i = FILETIME(), FILETIME(), FILETIME()
        ctypes.windll.kernel32.GetSystemTimes(
            ctypes.byref(k), ctypes.byref(u), ctypes.byref(i))
        return i.as_int(), k.as_int() + u.as_int()

    def sample(self) -> float:
        i0, t0 = self._idle
        wall = time.perf_counter() - self._t
        i1, t1 = self._idle_total()
        self._t, self._idle = time.perf_counter(), (i1, t1)
        if t1 == t0:
            return float("nan")
        return 1.0 - (i1 - i0) / (t1 - t0)


def same_result(a, b) -> bool:
    """逐段路径与原生路径的**语义等价**判定。

    比墙钟可靠得多：输出逐项相同 ⇒ 同一条代码路径，
    耗时差就只能来自机器噪声，而不是「抄错了函数体」。
    """
    import numpy as np

    ab, bb = getattr(a, "boxes", None), getattr(b, "boxes", None)
    at, bt = list(getattr(a, "txts", None) or []), list(getattr(b, "txts", None) or [])
    if (ab is None) != (bb is None) or len(at) != len(bt):
        return False
    if at != bt:
        return False
    if ab is None:
        return True
    return len(ab) == len(bb) and bool(np.allclose(np.asarray(ab), np.asarray(bb)))


def build_engine(model: str, threads: int):
    import logging

    logging.getLogger("rapidocr").setLevel(logging.ERROR)
    from rapidocr import RapidOCR

    det_name, rec_name = MODEL_FILES[model]
    mdir = MODEL_DIRS[model]
    for f in (det_name, rec_name, "ppocrv6_dict.txt"):
        if not (mdir / f).exists():
            raise SystemExit(f"缺权重 {mdir / f}（tiny 请先跑 scripts/ocrbench/fetch_tiny.py）")

    params: dict = {
        "Det.model_path": str(mdir / det_name),
        "Rec.model_path": str(mdir / rec_name),
        "Rec.rec_keys_path": str(mdir / "ppocrv6_dict.txt"),
    }
    if threads >= 0:
        # 图优化级别在 rapidocr 里是**硬编码** `ORT_ENABLE_ALL`
        # （inference_engine/onnxruntime/main.py:82），没有开关，
        # 所以「图优化已开启」不需要额外配，只需要在这里记一句。
        params["EngineConfig.onnxruntime.intra_op_num_threads"] = threads

    t0 = time.perf_counter()
    eng = RapidOCR(params=params)
    ctor_ms = (time.perf_counter() - t0) * 1000
    return eng, ctor_ms


def load_sessions(eng) -> dict:
    """分别计时三个会话的建立。

    ⚠ 这是**惰性**的：`RapidOCR.__init__` 一个模型都不建，
    所以「构造耗时」小得几乎没意义，真正的装载钱在这里。
    """
    out = {}
    for tag, fn in (
        ("det", eng._load_det_model),
        ("cls", eng._load_cls_model),
        ("rec", eng._load_rec_model),
    ):
        t0 = time.perf_counter()
        obj = fn()
        out[tag] = round((time.perf_counter() - t0) * 1000, 1)
        _INFER_TAG[id(obj.session)] = f"{tag}_infer"
    return out


def run_pipeline_steps(eng, arr) -> tuple[dict, object]:
    """照抄 `RapidOCR.__call__` 的六步，逐段计时。

    返回 `(分阶段耗时, 与原生同构的结果对象)`。结果对象用于语义等价自检。
    """
    _INFER_MS.clear()
    t0 = time.perf_counter()
    ori_img = eng.load_img(arr)
    img, op_record = eng.preprocess_img(ori_img)
    t_pre = time.perf_counter()

    cropped, det_res = eng.detect_and_crop(img, op_record)
    t_det = time.perf_counter()

    cls_list, cls_res = eng.cls_and_rotate(cropped)
    t_cls = time.perf_counter()

    rec_res = eng.recognize_txt(cls_list)
    t_rec = time.perf_counter()

    out = eng.build_final_output(ori_img, det_res, cls_res, rec_res, cropped, op_record)
    t_post = time.perf_counter()

    ms = lambda a, b: (b - a) * 1000  # noqa: E731
    # det_res.elapse 是 TextDetector.__call__ 内部的**整段**（预处理+推理+后处理），
    # 减去纯推理即得「det 的预处理 + DB 后处理」。
    det_total = (det_res.elapse or 0.0) * 1000
    rec_total = (rec_res.elapse or 0.0) * 1000
    cls_elapse = getattr(cls_res, "elapse", None)
    cls_total = (cls_elapse or 0.0) * 1000
    det_infer = _INFER_MS.get("det_infer", 0.0)
    rec_infer = _INFER_MS.get("rec_infer", 0.0)

    stages = {
        "preprocess": ms(t0, t_pre),
        "det_total": det_total,
        "det_infer": det_infer,
        "det_prepost": det_total - det_infer,
        "cls_total": cls_total,
        "cls_infer": _INFER_MS.get("cls_infer", 0.0),
        "rec_total": rec_total,
        "rec_infer": rec_infer,
        "rec_prepost": rec_total - rec_infer,
        "postprocess": ms(t_rec, t_post),
        "sum_of_stages": ms(t0, t_post),
    }
    # crop = detect_and_crop 的墙钟 − TextDetector 内部的 elapse
    # （detect_and_crop = 竖向 padding + TextDetector + crop_text_regions）
    stages["crop"] = ms(t_pre, t_det) - det_total
    return stages, out


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--model", choices=list(MODEL_DIRS), default="small")
    ap.add_argument("--threads", type=int, default=-1,
                    help="intra_op_num_threads；-1 = rapidocr 默认（ORT 自行决定）")
    ap.add_argument("--repeat", type=int, default=12, help="预热后的计时次数")
    ap.add_argument("--warmup", type=int, default=3)
    ap.add_argument("--scale", type=int, default=1, help="放大倍数；本轮实验固定 1×")
    ap.add_argument("--out", default="")
    args = ap.parse_args()
    if not args.out:
        args.out = str(OUT / f"latency-breakdown-{args.model}-t{args.threads}.json")

    install_infer_timer()
    import numpy as np
    from PIL import Image

    report: dict = {
        "generated": time.strftime("%Y-%m-%d %H:%M:%S"),
        "model": args.model,
        "intra_op_num_threads": args.threads,
        "scale": args.scale,
        "repeat": args.repeat,
        "warmup": args.warmup,
        "graph_optimization": "ORT_ENABLE_ALL（rapidocr 硬编码，无开关）",
        "cold": {},
        "warm": {},
    }

    # ---------------- 冷启动：三段分开 ----------------
    eng, ctor_ms = build_engine(args.model, args.threads)
    report["cold"]["engine_construct_ms"] = round(ctor_ms, 1)
    time.sleep(3)  # 冷却：上一段的磁盘/内存压力不污染下一段
    sess = load_sessions(eng)
    report["cold"]["session_build_ms"] = sess
    report["cold"]["session_build_total_ms"] = round(sum(sess.values()), 1)

    time.sleep(3)
    # **建立会话之后的第一次推理**——这才是「上一次报的那个数」真正该指的东西。
    first_calls = {}
    for name, spec in FRAMES.items():
        img = Image.open(spec["path"]).convert("RGB")
        if args.scale > 1:
            img = img.resize((img.width * args.scale, img.height * args.scale), Image.NEAREST)
        arr = np.array(img)[:, :, ::-1]
        t0 = time.perf_counter()
        res = eng(arr)
        first_calls[name] = {
            "ms": round((time.perf_counter() - t0) * 1000, 1),
            "boxes": 0 if getattr(res, "boxes", None) is None else len(res.boxes),
            "truth_lines": len(spec["centers"]),
        }
        time.sleep(3)
    report["cold"]["first_inference_after_load"] = first_calls

    # ---------------- 稳态：分阶段（逐段与原生交替采样） ----------------
    for name, spec in FRAMES.items():
        img = Image.open(spec["path"]).convert("RGB")
        if args.scale > 1:
            img = img.resize((img.width * args.scale, img.height * args.scale), Image.NEAREST)
        arr = np.array(img)[:, :, ::-1]

        for _ in range(args.warmup):
            eng(arr)
        time.sleep(3)

        acc: dict[str, list[float]] = {}
        e2e_call: list[float] = []
        eq_ok = True
        hit = 0
        busy = CpuBusy()

        def native_once() -> tuple[float, object]:
            t = time.perf_counter()
            res = eng(arr)
            return (time.perf_counter() - t) * 1000, res

        def steps_once() -> tuple[float, object]:
            st, out = run_pipeline_steps(eng, arr)
            for k, v in st.items():
                acc.setdefault(k, []).append(v)
            return st["sum_of_stages"], out

        for i in range(args.repeat):
            # **每轮顺序对调**：这台机器的噪声是 ±50%，顺序固定会让
            # 「谁先跑」系统性地便宜/贵。交替之后两侧吃到的漂移均摊。
            if i % 2 == 0:
                s_ms, s_out = steps_once()
                n_ms, n_out = native_once()
            else:
                n_ms, n_out = native_once()
                s_ms, s_out = steps_once()
            e2e_call.append(n_ms)
            if not same_result(s_out, n_out):
                eq_ok = False
            if i == 0:
                centers = spec["centers"]
                ys = []
                raw = getattr(n_out, "boxes", None)
                if raw is not None and len(raw) > 0:
                    for b in raw:
                        yy = [p[1] for p in b]
                        ys.append((min(yy) / args.scale, max(yy) / args.scale))
                hit = sum(1 for c in centers if any(b0 - 8 <= c <= b1 + 8 for b0, b1 in ys))

        if not eq_ok:
            raise SystemExit(
                f"[{name}] 等价性自检失败：逐段路径与原生 `RapidOCR.__call__` 的 "
                "boxes/txts 不一致 —— 抄错函数体了，不报数。"
            )

        cpu_busy = busy.sample()
        sum_med = statistics.median(acc["sum_of_stages"])
        e2e_med = statistics.median(e2e_call)
        spread = (max(e2e_call) - min(e2e_call)) / e2e_med

        report["warm"][name] = {
            "image_size": list(Image.open(spec["path"]).size),
            "truth_lines": len(spec["centers"]),
            "detected_boxes": hit,
            "detection": f"{hit}/{len(spec['centers'])}",
            "e2e_call": stat(e2e_call),
            "stages": {k: stat(v) for k, v in acc.items()},
            "semantic_equivalence": "boxes/txts 逐项相同",
            "wallclock_drift_pct": round(abs(sum_med - e2e_med) / e2e_med * 100, 1),
            "e2e_spread_pct": round(spread * 100, 1),
            "system_cpu_busy_during_sampling": round(cpu_busy, 3),
        }
        print(f"[{name}] e2e 中位 {e2e_med:.0f} ms  P95 {p95(e2e_call):.0f} ms  "
              f"检出 {hit}/{len(spec['centers'])}  "
              f"噪声幅度(max-min)/中位 {spread:.0%}  系统占用 {cpu_busy:.0%}", flush=True)
        time.sleep(5)  # 冷却

    Path(args.out).write_text(json.dumps(report, ensure_ascii=False, indent=1),
                              encoding="utf-8")
    print(f"-> {args.out}")

    print("\n## 冷启动拆解")
    print(f"  引擎构造 RapidOCR()          {report['cold']['engine_construct_ms']} ms")
    for k, v in sess.items():
        print(f"  会话建立 {k:<4}               {v} ms")
    print(f"  会话建立合计                 {report['cold']['session_build_total_ms']} ms")
    for k, v in first_calls.items():
        print(f"  建立后首推 {k:<14}      {v['ms']} ms  检出 {v['boxes']}/{v['truth_lines']}")

    for name in report["warm"]:
        print(f"\n## {name} 稳态分阶段（n={args.repeat}）")
        print("| 阶段 | 中位 ms | P95 ms |")
        print("|---|---|---|")
        for k, v in report["warm"][name]["stages"].items():
            print(f"| {k} | {v['median_ms']} | {v['p95_ms']} |")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
