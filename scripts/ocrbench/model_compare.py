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
"""tiny vs small：同一批图、同一把尺子，只换模型规格。

```
target\\bench-venv\\Scripts\\python.exe scripts/ocrbench/model_compare.py --model tiny
target\\bench-venv\\Scripts\\python.exe scripts/ocrbench/model_compare.py --model small
```

## 口径

- **尺子复用 `bench_lib`**（规范化、编辑距离、纵向带 50% 重叠配对、漏检阈值 0.5），
  与 `ocrbench.py` 逐字同一套。两条规格拿到的输入图与判据完全一致，
  唯一允许不同的是权重文件。
- **放大倍数固定 1×**：上一轮实测 PP-OCRv6 small 在 1× 已零漏检，
  2× 只是更慢，所以这轮不引入第二个变量。
- **真帧只报检出率，不报文本**：检出率是不看内容就能数的指标，
  避免把真人聊天内容写进产物。
- **耗时每档都跑两帧 × `--repeat` 次**（预热后），报中位数与 P95；
  合成语料 60 张各跑 1 次，只用于准确率，顺带给出它的耗时分布
  （⚠ 那是「每张图首调」，不是稳态，别当延迟指标读）。

## 一次只动一个变量

`--model` 是唯一的轴。线程数固定为 rapidocr 默认（-1），
不在这个脚本里扫线程——那是 `latency_breakdown.py --threads` 的活，
两件事混在一起就没法归因。
"""

from __future__ import annotations

import argparse
import json
import statistics
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
OUT = ROOT / "target" / "probe" / "ocrbench"
sys.path.insert(0, str(Path(__file__).resolve().parent))

from bench_lib import Hyp, align, score  # noqa: E402
from latency_breakdown import MODEL_DIRS, MODEL_FILES, p95  # noqa: E402

TRUTH = OUT / "truth.json"

# 真值中心按帧写死（1× 坐标），出处见 latency_breakdown.py 的 FRAMES。
from latency_breakdown import FRAMES  # noqa: E402

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


def load_arr(path: Path):
    import numpy as np
    from PIL import Image

    img = Image.open(path).convert("RGB")
    return np.array(img)[:, :, ::-1]


def fmt(v, pct=False):
    if v is None:
        return "n/a"
    return f"{v * 100:.2f}%" if pct else f"{v:.4f}"


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--model", choices=list(MODEL_DIRS), default="small")
    ap.add_argument("--repeat", type=int, default=10, help="真帧每帧的计时次数")
    ap.add_argument("--warmup", type=int, default=3)
    ap.add_argument("--out", default="")
    args = ap.parse_args()
    if not args.out:
        args.out = str(OUT / f"compare-{args.model}.json")

    import logging

    logging.getLogger("rapidocr").setLevel(logging.ERROR)
    from rapidocr import RapidOCR

    det_name, rec_name = MODEL_FILES[args.model]
    mdir = MODEL_DIRS[args.model]
    for f in (det_name, rec_name, "ppocrv6_dict.txt"):
        if not (mdir / f).exists():
            raise SystemExit(f"缺权重 {mdir / f}")

    t0 = time.perf_counter()
    eng = RapidOCR(params={
        "Det.model_path": str(mdir / det_name),
        "Rec.model_path": str(mdir / rec_name),
        "Rec.rec_keys_path": str(mdir / "ppocrv6_dict.txt"),
    })
    ctor_ms = (time.perf_counter() - t0) * 1000

    report = {
        "generated": time.strftime("%Y-%m-%d %H:%M:%S"),
        "model": args.model,
        "weights": {n: (mdir / n).stat().st_size for n in (det_name, rec_name)},
        "scale": 1,
        "engine_construct_ms": round(ctor_ms, 1),
        "real_frames": {},
        "corpus": {},
    }

    # ---------------- 真帧：检出率 + 延迟 ----------------
    for name, spec in FRAMES.items():
        arr = load_arr(spec["path"])
        for _ in range(args.warmup):
            eng(arr)
        time.sleep(2)

        times: list[float] = []
        hit = 0
        boxes_n = 0
        for i in range(args.repeat):
            t = time.perf_counter()
            res = eng(arr)
            times.append((time.perf_counter() - t) * 1000)
            raw = getattr(res, "boxes", None)
            if i == 0 and raw is not None and len(raw) > 0:
                spans = []
                for b in raw:
                    yy = [p[1] for p in b]
                    spans.append((min(yy), max(yy)))
                boxes_n = len(spans)
                hit = sum(1 for c in spec["centers"]
                          if any(b0 - 8 <= c <= b1 + 8 for b0, b1 in spans))
        report["real_frames"][name] = {
            "truth_lines": len(spec["centers"]),
            "boxes": boxes_n,
            "detection": f"{hit}/{len(spec['centers'])}",
            "detection_rate": hit / len(spec["centers"]),
            "n": len(times),
            "median_ms": round(statistics.median(times), 1),
            "p95_ms": round(p95(times), 1),
            "min_ms": round(min(times), 1),
            "max_ms": round(max(times), 1),
        }
        print(f"[真帧 {name}] 检出 {hit}/{len(spec['centers'])}  "
              f"中位 {statistics.median(times):.0f} ms  P95 {p95(times):.0f} ms  "
              f"(n={len(times)})", flush=True)
        time.sleep(3)

    # ---------------- 合成语料：CER / 漏检 ----------------
    truth = json.loads(TRUTH.read_text(encoding="utf-8"))
    all_j: list = []
    corpus_times: list[float] = []
    for n, im in enumerate(truth["images"], 1):
        arr = load_arr(ROOT / im["path"])
        t = time.perf_counter()
        res = eng(arr)
        corpus_times.append((time.perf_counter() - t) * 1000)
        raw = getattr(res, "boxes", None)
        hyps: list[Hyp] = []
        if raw is not None and len(raw) > 0:
            txts = list(getattr(res, "txts", None) or [])
            for b, txt in zip(raw, txts):
                yy = [p[1] for p in b]
                hyps.append(Hyp(txt, int(min(yy)), int(max(yy))))
        all_j.extend(align(im["lines"], hyps))
        if n % 10 == 0:
            print(f"  语料 {n}/{len(truth['images'])}", flush=True)
        time.sleep(0.2)  # 轻冷却，别让上一张的分配器压力压到下一张

    s = score(all_j)
    report["corpus"] = {
        "images": len(truth["images"]),
        "n_gt_lines": s["n_gt_lines"],
        "gt_chars": s["gt_chars"],
        "cer": s["cer"],
        "miss_rate": s["miss_rate"],
        "geom_miss_rate": s["geom_miss_rate"],
        "per_image_ms_median": round(statistics.median(corpus_times), 1),
        "per_image_ms_p95": round(p95(corpus_times), 1),
        "per_image_ms_note": "每张图各跑 1 次，是「首调」不是稳态",
    }
    Path(args.out).write_text(json.dumps(report, ensure_ascii=False, indent=1),
                              encoding="utf-8")

    print(f"\n## {args.model} @1×")
    print(f"权重 det {(mdir / det_name).stat().st_size / 1e6:.1f} MB / "
          f"rec {(mdir / rec_name).stat().st_size / 1e6:.1f} MB")
    print(f"合成语料 {len(truth['images'])} 图 / {s['n_gt_lines']} 行 / {s['gt_chars']} 字")
    print(f"  CER {fmt(s['cer'])}  漏检率 {fmt(s['miss_rate'], True)}  "
          f"几何漏检率 {fmt(s['geom_miss_rate'], True)}")
    print(f"  单图耗时中位 {statistics.median(corpus_times):.0f} ms  "
          f"P95 {p95(corpus_times):.0f} ms（每图 1 次，非稳态）")
    print(f"-> {args.out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
