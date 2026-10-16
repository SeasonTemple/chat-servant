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
"""OCR 引擎对照评测的唯一入口。

    G:\\tools\\uv\\uv.exe run python scripts/ocrbench/ocrbench.py

跑完产出 `target/probe/ocrbench/report.{json,md}`。

## 两条路线

- **A = WinRT**（`Windows.Media.Ocr`）：项目自己的 Rust 实现，
  用 `xtask ocr` 驱动，引擎由 `CHATSERVANT_OCR=winrt` 选定。
- **B = PP-OCRv6 small**（ONNX Runtime）：RapidOCR 3.10.0，
  权重是本仓 `target/models/ppocrv6-small/` 下那三个文件。

## 同一把尺子

放大倍数、规范化、编辑距离、对齐规则全部来自 `bench_lib` 与 `SCALES`，
两条路线拿到的输入图与判据完全一致。唯一允许不同的是推理引擎本身。

## 冷/热的定义（两条路线不共用同一个口径，必须分别说）

- **A**：每个 (图, 倍数) 是一个**独立进程**，所以该进程的第 1 次调用
  就是这个引擎的冷启动（要初始化 WinRT 引擎）。稳态 = 第 2~5 次的中位数。
- **B**：整个评测在**同一个 Python 进程**里跑（生产里 OCR 跑在长驻进程），
  引擎在第一条作业之前就建好。第 1 条作业的第 1 次调用是冷启动，
  之后每条作业的第 2~N 次中位数是稳态。
- **稳态计时子集**：只有 15 张图跑重复调用（见 `TIMING_SUBSEED_STRIDE`），
  其余只跑一次只取准确率。理由见 ADR：全量 4 遍会把机时翻四倍，
  而这批图里 det 会自行升采样，重复调用的收益只体现在延迟统计上。
"""

from __future__ import annotations

import json
import os
import re
import statistics
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
MODELS = ROOT / "target" / "models" / "ppocrv6-small"
OUT = ROOT / "target" / "probe" / "ocrbench"
RESULTS = OUT / "results"
XTASK = ROOT / "target" / "release" / "xtask.exe"

SCALES = [1, 2]
SCALE4_SEED_ONLY = 0  # 只给 seed==0 的图补 scale=4，量「4× 反而更差」那条
# 执行顺序 = 产品实际用的档位优先。2× 是 observe 里放大后送 OCR 的那一档。
SCALE_ORDER = [2, 1, 4]
REPEAT_TIMING = 5
TIMING_SUBSEED_STRIDE = 2  # 每 2 条作业挑 1 条进计时子集 -> 15 条

sys.path.insert(0, str(Path(__file__).resolve().parent))
from bench_lib import (  # noqa: E402
    MISS_SIM,
    Hyp,
    align,
    normalize,
    normalize_strict,
    score,
)

RE_ITER = re.compile(r"^第 (\d+)/(\d+) 遍：OCR (\d+) ms")
RE_WARM = re.compile(r"^冷启动 (\d+) ms\s+稳态 min/中位/max = (\d+)/(\d+)/(\d+) ms")
RE_LINE = re.compile(r"^\s*(\d+) \| y=\s*(\d+) h=\s*(\d+) \| (.*)$")


def build_jobs(truth: dict) -> list[dict]:
    """决定要跑哪些 (图, 倍数, 重复) 作业。顺序固定，可复现。"""
    jobs: list[dict] = []
    for im in truth["images"]:
        for s in SCALES:
            jobs.append({"id": im["id"], "path": im["path"], "scale": s})
        if im["seed"] == SCALE4_SEED_ONLY:
            jobs.append({"id": im["id"], "path": im["path"], "scale": 4})
    # 计时子集：在 scale==2 的作业里按步长挑。
    timing_candidates = [j for j in jobs if j["scale"] == 2]
    timing_ids = {j["id"] for j in timing_candidates[::TIMING_SUBSEED_STRIDE]}
    for j in jobs:
        j["repeat"] = REPEAT_TIMING if (j["id"] in timing_ids and j["scale"] == 2) else 1
        j["timing"] = j["repeat"] > 1
    # **按档位排序，2× 优先。**
    # 2× 是产品实际用的那一档；万一跑到一半被迫中止，
    # 已出的数也是最该看的那一档，而不是一堆 1× 配上没跑过的 2×。
    # 计时子集的选取在上面已经定好了，排序不影响它。
    jobs.sort(key=lambda j: (SCALE_ORDER.index(j["scale"]), j["id"]))
    return jobs


# --------------------------------------------------------------------------
# 路线 A：WinRT，经由 xtask ocr
# --------------------------------------------------------------------------


def run_route_a(jobs: list[dict]) -> list[dict]:
    if not XTASK.exists():
        sys.exit(
            f"缺 {XTASK}。本脚本不自己编译（会抢 target/ 锁），"
            "请先由父代理 `cargo build --release -p xtask`。"
        )
    env = {**os.environ, "CHATSERVANT_OCR": "winrt"}
    recs = []
    for j in jobs:
        p = subprocess.run(
            [
                str(XTASK), "ocr", j["path"],
                "--scale", str(j["scale"]), "--repeat", str(j["repeat"]),
            ],
            capture_output=True,
            cwd=ROOT,
            env=env,
        )
        if p.returncode != 0:
            recs.append({**j, "error": p.stderr.decode("utf-8", "replace")[-400:]})
            continue
        text = p.stdout.decode("utf-8", "replace")
        ms = [int(m.group(3)) for m in (RE_ITER.match(l.strip()) for l in text.splitlines()) if m]
        hyps = []
        for line in text.splitlines():
            m = RE_LINE.match(line)
            if not m:
                continue
            y0, h = int(m.group(2)), int(m.group(3))
            hyps.append(Hyp(m.group(4), y0, y0 + h))
        recs.append({**j, "ms": ms, "hyps": [(h.text, h.y0, h.y1) for h in hyps]})
        print(f"  A {j['id']} x{j['scale']} {len(ms)} calls {ms}", flush=True)
    return recs


# --------------------------------------------------------------------------
# 路线 B：PP-OCRv6 small（ONNX Runtime），单进程长驻
# --------------------------------------------------------------------------


def run_route_b(jobs: list[dict]) -> list[dict]:
    import logging

    import numpy as np
    from PIL import Image

    logging.getLogger("rapidocr").setLevel(logging.ERROR)
    from rapidocr import RapidOCR

    engine = RapidOCR(
        params={
            "Det.model_path": str(MODELS / "PP-OCRv6_det_small.onnx"),
            "Rec.model_path": str(MODELS / "PP-OCRv6_rec_small.onnx"),
            "Rec.rec_keys_path": str(MODELS / "ppocrv6_dict.txt"),
        }
    )
    first_call_ms: list[float] = []
    recs = []
    for j in jobs:
        # **最近邻**，与 `xtask ocr` 的 `upscaled_nearest` 对齐。
        # 用双三次会让路线 B 拿到更平滑的输入，等于给它单独开小灶。
        img = Image.open(ROOT / j["path"]).convert("RGB")
        if j["scale"] > 1:
            img = img.resize(
                (img.width * j["scale"], img.height * j["scale"]),
                Image.NEAREST,
            )
        arr = np.array(img)[:, :, ::-1]  # rapidocr 吃 BGR

        ms = []
        hyps = []
        for _ in range(j["repeat"]):
            t0 = time.perf_counter()
            res = engine(arr)
            ms.append(round((time.perf_counter() - t0) * 1000))
            boxes, txts = getattr(res, "boxes", None), getattr(res, "txts", None)
            hyps = []
            if boxes is not None:
                for box, txt in zip(boxes, txts or []):
                    ys = [p[1] for p in box]
                    hyps.append([txt, int(min(ys)), int(max(ys))])
        rec: dict = {**j, "ms": ms, "hyps": hyps}
        # 整个评测只建一次引擎（生产里 OCR 跑在长驻进程），
        # 所以**只有进程内第一次调用是冷启动**。后面每条作业的 ms[0]
        # 已经是热调用了——把它当冷启动报出去会得到一个假的「冷」数字。
        if not first_call_ms:
            first_call_ms = ms[:]
            rec["process_cold"] = ms[0]
        recs.append(rec)
        print(f"  B {j['id']} x{j['scale']} {len(ms)} calls {ms}", flush=True)
    return recs


# --------------------------------------------------------------------------
# 汇总
# --------------------------------------------------------------------------


def scaled_lines(im: dict, s: int) -> list[dict]:
    """把 ground truth 的行带换算到**放大后**的坐标系。

    ⚠ 这是踩过的坑：路线 A 的 `xtask ocr --scale n` 先把帧放大，
    识别结果里的 `y` 是**放大后**的坐标；而路线 B 是我自己缩放图，
    出来的也是放大后坐标。两边都必须拿同一坐标系里的 ground truth 去配对，
    否则放大倍数一上来就「一个框都对不上」，CER 直接飙到 ~1.0，
    而实际上识别得好得很。
    """
    if s == 1:
        return im["lines"]
    return [{**gt, "band": [c * s for c in gt["band"]]} for gt in im["lines"]]


def summarize(truth: dict, jobs: list[dict], recs: list[dict], route: str) -> dict:
    by_id = {im["id"]: im for im in truth["images"]}
    rec_by_key = {(r["id"], r["scale"]): r for r in recs if "ms" in r}
    out: dict = {"route": route, "errors": [r for r in recs if "error" in r], "by_scale": {}}

    for s in sorted({j["scale"] for j in jobs}):
        subset = [j for j in jobs if j["scale"] == s]
        acc: dict = {}
        for px in sorted({by_id[j["id"]]["font_px"] for j in subset}):
            acc[px] = {"edits": 0, "gt_chars": 0, "n_lines": 0, "miss": 0, "gmiss": 0, "edits_s": 0}
        tot = {"edits": 0, "gt_chars": 0, "n_lines": 0, "miss": 0, "gmiss": 0, "edits_s": 0}
        edits_s = 0
        hyp_lines = 0
        for j in subset:
            r = rec_by_key.get((j["id"], s))
            if not r:
                continue
            im = by_id[j["id"]]
            px = im["font_px"]
            hyps = [Hyp(*h) for h in r["hyps"]]
            js = align(scaled_lines(im, s), hyps, normalize)
            hyp_lines += len(r["hyps"])
            for jd in js:
                g = normalize(jd.gt)
                if not g:
                    continue
                edits_s += levenshtein_gs(jd, r["hyps"])
                for bucket in (tot, acc[px]):
                    bucket["edits"] += jd.edit
                    bucket["edits_s"] += levenshtein_gs(jd, r["hyps"])
                    bucket["gt_chars"] += len(g)
                    bucket["n_lines"] += 1
                    bucket["miss"] += jd.sim < MISS_SIM
                    bucket["gmiss"] += not jd.geom_hit

        # 冷/热分开：只在计时子集上有意义
        tsub = [r for r in recs if "ms" in r and r["scale"] == s and r["repeat"] > 1]
        cold = [r["ms"][0] for r in tsub]
        warm = [m for r in tsub for m in r["ms"][1:]]

        # 路线 B 的「冷」与路线 A 不是一个东西。
        # A：每张图一个新进程 ⇒ ms[0] 确实是那次进程的首调。
        # B：整个评测共用一个长驻进程 ⇒ 每条作业的 ms[0] 都已经是热调用，
        #    唯一的冷启动是**进程内第一次调用**。两种口径混用就会报出
        #    一个「看起来像冷启动、其实不冷」的数字，于是得出
        #    「这个引擎冷启动一点都不贵」的反向结论。
        proc_cold = next(
            (
                r["process_cold"]
                for r in recs
                if "ms" in r and r["scale"] == s and "process_cold" in r
            ),
            None,
        )
        cold_ms = proc_cold if proc_cold is not None else (
            statistics.median(cold) if cold else None
        )
        cold_basis = (
            "进程内第一次调用" if proc_cold is not None else "计时子集各作业首调的中位数"
        )

        # 计时也按字号分档。耗时是随字号变的（字多→框多→rec 批更大），
        # 只有一个总体中位数的话，读者会把它当成「对任意字号都成立」的数字。
        #
        # ⚠ 这里记的是**该字号每张图的首调**中位数，不是冷启动：
        # 计时子集里的图在进程里都不是第一个被处理的，冷启动只有一次。
        tpx: dict = {}
        for r in tsub:
            px = by_id[r["id"]]["font_px"]
            tpx.setdefault(px, {"first": [], "warm": []})
            tpx[px]["first"].append(r["ms"][0])
            tpx[px]["warm"].extend(r["ms"][1:])
        for px, tv in tpx.items():
            acc[px]["first_call_ms_median"] = statistics.median(tv["first"])
            acc[px]["steady_ms_median"] = statistics.median(tv["warm"])
            acc[px]["timing_images"] = len(tv["first"])

        def agg(b):
            return {
                "cer": b["edits"] / b["gt_chars"] if b["gt_chars"] else None,
                "miss_rate": b["miss"] / b["n_lines"] if b["n_lines"] else None,
                "geom_miss_rate": b["gmiss"] / b["n_lines"] if b["n_lines"] else None,
                "n_gt_lines": b["n_lines"],
                "gt_chars": b["gt_chars"],
                "cer_strict_ws": b["edits_s"] / b["gt_chars"] if b["gt_chars"] else None,
                "first_call_ms_median": b.get("first_call_ms_median"),
                "steady_ms_median": b.get("steady_ms_median"),
                "timing_images": b.get("timing_images"),
            }

        out["by_scale"][str(s)] = {
            "images": len(subset),
            "n_gt_lines": tot["n_lines"],
            "gt_chars": tot["gt_chars"],
            "edits": tot["edits"],
            "cer": tot["edits"] / tot["gt_chars"] if tot["gt_chars"] else None,
            "cer_strict_ws": edits_s / tot["gt_chars"] if tot["gt_chars"] else None,
            "miss_rate": tot["miss"] / tot["n_lines"] if tot["n_lines"] else None,
            "geom_miss_rate": tot["gmiss"] / tot["n_lines"] if tot["n_lines"] else None,
            "hyp_lines": hyp_lines,
            "gt_lines": tot["n_lines"],
            "overdetect_ratio": hyp_lines / tot["n_lines"] if tot["n_lines"] else None,
            "timing_subset_images": len(tsub),
            "cold_ms_median": cold_ms,
            "cold_basis": cold_basis,
            "steady_ms_median": statistics.median(warm) if warm else None,
            "steady_ms_min": min(warm) if warm else None,
            "steady_ms_max": max(warm) if warm else None,
            "by_font_px": {str(px): agg(b) for px, b in acc.items()},
        }
    return out


def levenshtein_gs(jd, hyps: list) -> int:
    """严格空白口径下的编辑距离。

    需要重新配对（严格口径下空格算字符），所以不能在 `align` 里顺带算。
    这里复用 `align` 的几何判据，只换规范化函数。
    """
    js = align([{"text": jd.gt, "band": jd.band}], [Hyp(*h) for h in hyps], normalize_strict)
    return js[0].edit if js else 0


def build_report(truth: dict, jobs: list[dict], summaries: dict) -> dict:
    return {
        "generated": time.strftime("%Y-%m-%d %H:%M:%S"),
        "normalization_primary": "NFKC + 删光所有 Unicode 空白 + lower()",
        "normalization_secondary": "NFKC + 连续空白压成 1 个空格 + 去首尾 + lower()",
        "miss_similarity_threshold": MISS_SIM,
        "geography": "纵向带相交覆盖较矮一方 >= 50% 视为对应",
        "scales": SCALES + [4],
        "xtask_mtime": time.strftime("%Y-%m-%d %H:%M:%S", time.localtime(XTASK.stat().st_mtime))
        if XTASK.exists() else None,
        "routes": summaries,
    }


def main() -> None:
    RESULTS.mkdir(parents=True, exist_ok=True)
    truth_path = OUT / "truth.json"
    if not truth_path.exists():
        print("语料不存在，先生成…", flush=True)
        subprocess.run([sys.executable, str(Path(__file__).parent / "gen_corpus.py")],
                       check=True, cwd=ROOT)
    truth = json.loads(truth_path.read_text(encoding="utf-8"))
    jobs = build_jobs(truth)

    argv = sys.argv[1:]
    # --report-only：只读已存的 results/*.json 重算报表，**不跑任何推理**。
    # 用途是改口径/改表格格式时不必重付一次机时——报表是纯函数。
    report_only = "--report-only" in argv
    which = [a for a in argv if a in ("a", "b")] or ["a", "b"]

    print(f"作业 {len(jobs)} 条（计时子集 {sum(j['timing'] for j in jobs)} 条）"
          f"{'｜--report-only：不跑推理' if report_only else ''}", flush=True)

    summaries = {}
    for key, fname, route_name, runner in (
        ("a", "route_a.json", "A_winrt", run_route_a),
        ("b", "route_b.json", "B_ppocrv6small", run_route_b),
    ):
        fp = RESULTS / fname
        if report_only:
            if fp.exists():
                recs = json.loads(fp.read_text(encoding="utf-8"))
                summaries[route_name] = summarize(truth, jobs, recs, route_name)
                print(f"  读回 {fname}（{len(recs)} 条）", flush=True)
            continue
        if key not in which:
            continue
        print(f"路线 {key.upper()} …", flush=True)
        recs = runner(jobs)
        fp.write_text(json.dumps(recs, ensure_ascii=False), encoding="utf-8")
        summaries[route_name] = summarize(truth, jobs, recs, route_name)

    if not summaries:
        sys.exit("没有可汇总的路线结果（--report-only 需要先跑过一次）")
    report = build_report(truth, jobs, summaries)
    (OUT / "report.json").write_text(
        json.dumps(report, ensure_ascii=False, indent=1), encoding="utf-8")
    write_md(report)
    print(f"\n-> {OUT / 'report.md'}")


def fmt(v, pct=False):
    if v is None:
        return "n/a"
    return f"{v * 100:.2f}%" if pct else f"{v:.4f}"


def write_md(rep: dict) -> None:
    L = [
        "# OCR 引擎对照评测（自动生成，勿手改）",
        "",
        f"生成时间：{rep['generated']}",
        f"主口径规范化：{rep['normalization_primary']}",
        f"对照口径规范化：{rep['normalization_secondary']}",
        f"漏检阈值：归一化相似度 < {rep['miss_similarity_threshold']}",
        f"配对判据：{rep['geography']}",
        "",
        "## 总体（按放大倍数）",
        "",
        "| 路线 | 倍数 | 图数 | CER | CER(严格空白) | 漏检率 | 几何漏检率 | 冷启动 ms | 稳态 ms | 稳态 min/max ms |",
        "|---|---|---|---|---|---|---|---|---|---|",
    ]
    for name, r in rep["routes"].items():
        for s, m in sorted(r["by_scale"].items(), key=lambda kv: int(kv[0])):
            L.append(
                f"| {name} | {s}x | {m['images']} | {fmt(m['cer'])} | "
                f"{fmt(m['cer_strict_ws'])} | {fmt(m['miss_rate'], True)} | "
                f"{fmt(m['geom_miss_rate'], True)} | "
                f"{m['cold_ms_median']} | {m['steady_ms_median']} | "
                f"{m['steady_ms_min']}/{m['steady_ms_max']} |"
            )
        basis = {m.get("cold_basis") for m in r["by_scale"].values() if m.get("cold_basis")}
        if basis:
            L.append(f"| | | | | | | | 冷启动口径：{'; '.join(sorted(basis))} | | |")
    L += ["", "## 按字号分档", "",
          "`timing 图` = 该字号里进了计时子集的图数；计时列只在计时子集上有意义。",
          "`首调 ms` 是**该字号每张图的首调**中位数，**不是冷启动**"
          "（冷启动整个进程只有一次，见上表口径）。", ""]
    for name, r in rep["routes"].items():
        for s, m in sorted(r["by_scale"].items(), key=lambda kv: int(kv[0])):
            L += [
                f"### {name} · {s}x",
                "",
                "| 字号 | GT 行 | CER | CER(严格空白) | 漏检率 | 几何漏检率 | 首调 ms | 稳态 ms | timing 图 |",
                "|---|---|---|---|---|---|---|---|---|",
            ]
            for px, b in sorted(m["by_font_px"].items(), key=lambda kv: int(kv[0])):
                L.append(
                    f"| {px}px | {b['n_gt_lines']} | {fmt(b['cer'])} | "
                    f"{fmt(b['cer_strict_ws'])} | {fmt(b['miss_rate'], True)} | "
                    f"{fmt(b['geom_miss_rate'], True)} | {b['first_call_ms_median']} | "
                    f"{b['steady_ms_median']} | {b['timing_images']} |"
                )
            L.append("")
    (OUT / "report.md").write_text("\n".join(L) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
