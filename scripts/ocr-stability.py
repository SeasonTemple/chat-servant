#!/usr/bin/env python3
"""量「同一帧、换口径、结果不一致」的行 —— 识别稳定性的量具。

## 为什么要有这个

票 96 的错字（源字是汉字、输出成了片假名）在真机上复现了，但「发生率」一直
说不清：唯一的信号是 merged_pair_suspect 那两条诊断，而它判的是**形状**。
拿「命中 4 / 19 条消息」去说错字率，等于拿体温计当验血报告。

这台机器禁止输入注入，所以「让人多点几屏、再数命中」这条路走不通。但**引擎自己
稳不稳是可以量的**：同一帧、只换放大倍数或重采样方式，跑几遍，逐行比对。
两次结果不一致的行，就是「引擎自己都没把握」的行 —— 它不需要人手、不需要真值，
而且能天天跑。

⚠ 它量的是**不稳定性**，不是**错误率**：稳定地读错（每次都错成同一个字）
不会被它抓出来。真值只能靠人。但作为「引擎在这块到底稳不稳」的指标，它比数
诊断条数硬。

## 用法

    uv run --no-project python scripts/ocr-stability.py target/probe/wx.png
    uv run --no-project python scripts/ocr-stability.py target/probe/wx.png --passes 1:nearest,2:nearest,2:bilinear,3:nearest

默认口径（--passes）：1:nearest,2:nearest,2:bilinear。
1:nearest 是诊断基准（字形一字不差），另外两个是「同一帧换个看法」。

## 输出落在哪

详细报告**含真实聊天文本**，所以只写到 target/probe/ocr-stability/（gitignored）。
终端只打计数与位置 —— 硬约束 3：文本不许进提交信息、注释、夹具。
--print-text 会把差异行打到终端（本机调试用，别把终端记录粘进任何文件）。
"""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
from pathlib import Path

# Windows 的 stdout 默认 cp1252，打印中文会 UnicodeEncodeError（CI 的 runner 尤其）。
# 别指望调用方记得设 PYTHONIOENCODING —— 样板见 scripts/targetgc.py。
for _s in (sys.stdout, sys.stderr):
    if hasattr(_s, "reconfigure"):
        _s.reconfigure(encoding="utf-8", errors="replace")

ROOT = Path(__file__).resolve().parent.parent

LINE_RE = re.compile(
    r"^\s*\d+ \| x=\s*(-?\d+) w=\s*(\d+) y=\s*(\d+) h=\s*(\d+) \| (.*)$"
)

# perceive 的汇总行：  [对方] (79,407)-(169,427)  整行文本
PROD_RE = re.compile(r"^\s*\[(\S+)\] \((\d+),(\d+)\)-\((\d+),(\d+)\)\s+(.*)$")


def run_pass(frame: Path, scale: int, resample: str) -> list[dict]:
    """跑一遍 OCR，返回它打出来的每一行（带几何）。"""
    cmd = [
        "cargo", "run", "-q", "-p", "xtask", "--", "ocr", str(frame),
        "--scale", str(scale), "--resample", resample,
    ]
    proc = subprocess.run(cmd, capture_output=True, cwd=ROOT)
    if proc.returncode != 0:
        raise SystemExit(
            f"OCR 跑失败了（scale={scale} resample={resample}，exit={proc.returncode}）：\n"
            + proc.stderr.decode("utf-8", "replace")[-2000:]
        )
    rows = []
    for raw in proc.stdout.decode("utf-8", "replace").splitlines():
        m = LINE_RE.match(raw)
        if not m:
            continue
        x, w, y, h, body = m.groups()
        rows.append({"x": int(x), "w": int(w), "y": int(y), "h": int(h), "text": body.strip()})
    if scale > 1:
        # --scale 是**识别前**把图放大（见 xtask 的 load_frame），所以它打的坐标在放大空间里。
        # 折回原图再比，否则两遍的行永远配不上（实测 166 行配错对）。
        for r in rows:
            r["x"] = round(r["x"] / scale)
            r["y"] = round(r["y"] / scale)
            r["w"] = round(r["w"] / scale)
            r["h"] = round(r["h"] / scale)
    return rows


def align(base: list[dict], other: list[dict], tol_y: int = 6) -> list[tuple]:
    """按几何把两遍的行配对。同一个字框在两遍里位置几乎不动，所以按 (y, x) 配。"""
    pairs = []
    used = set()
    for a in base:
        best = None
        best_d = None
        for i, b in enumerate(other):
            if i in used or abs(b["y"] - a["y"]) > tol_y:
                continue
            d = abs(b["x"] - a["x"])
            if best_d is None or d < best_d:
                best, best_d = i, d
        if best is None:
            pairs.append((a, None))
        else:
            used.add(best)
            pairs.append((a, other[best]))
    for i, b in enumerate(other):
        if i not in used:
            pairs.append((None, b))
    return pairs


def first_diff(a: str, b: str):
    for i, (ca, cb) in enumerate(zip(a, b)):
        if ca != cb:
            return i
    if len(a) != len(b):
        return min(len(a), len(b))
    return None


def main(argv: list[str]) -> int:
    ap = argparse.ArgumentParser(description="同一帧换口径跑多遍，找引擎自己不一致的行")
    ap.add_argument("frame", type=Path, help="要量的 PNG")
    ap.add_argument(
        "--passes",
        default="1:nearest,2:nearest,2:bilinear",
        help="逗号分隔的 scale:resample 组合",
    )
    ap.add_argument("--print-text", action="store_true", help="把差异行打到终端（本机调试用）")
    args = ap.parse_args(argv)

    frame = args.frame
    if not frame.exists():
        raise SystemExit(f"帧不存在：{frame}")

    specs = []
    for item in args.passes.split(","):
        scale_s, _, resample = item.partition(":")
        specs.append((int(scale_s), resample or "nearest"))

    passes = []
    for scale, resample in specs:
        rows = run_pass(frame, scale, resample)
        passes.append((f"{scale}:{resample}", rows))
        print(f"口径 {scale}:{resample} ⇒ {len(rows)} 行")

    base_name, base = passes[0]
    unstable = []
    missing = []
    for other_name, other in passes[1:]:
        for a, b in align(base, other):
            if a is None or b is None:
                missing.append((other_name, a, b))
                continue
            if a["text"] != b["text"]:
                unstable.append((a, b, other_name, first_diff(a["text"], b["text"])))

    total = len(base)
    unstable_lines = {id(x[0]) for x in unstable}
    print()
    print(f"基准口径 {base_name}：{total} 行")
    if total:
        print(f"换口径后**文本变了**的行：{len(unstable_lines)}（{len(unstable_lines)}/{total}）")
    else:
        print("基准一条行都没有")
    print(f"只在一边出现的行（框没配对上）：{len(missing)}")
    print("⚠ 这是**不稳定性**，不是错误率：稳定地读错抓不出来。")

    outdir = ROOT / "target" / "probe" / "ocr-stability"
    outdir.mkdir(parents=True, exist_ok=True)
    out = outdir / (frame.stem + ".txt")
    with out.open("w", encoding="utf-8", newline="\n") as f:
        f.write(f"# {frame}  基准 {base_name}（{total} 行）\n")
        f.write("# 这个文件含真实聊天文本，只许留在 target/ 下\n\n")
        for a, b, other_name, at in unstable:
            f.write(f"[{other_name}] 第 {at} 字起不同\n")
            f.write(f"  {base_name}: y={a['y']} x={a['x']} {a['text']}\n")
            f.write(f"  {other_name}: y={b['y']} x={b['x']} {b['text']}\n\n")
        for other_name, a, b in missing:
            f.write(f"[{other_name}] 框没配对：base={a} other={b}\n")
    print(f"详细报告（含文本）：{out.relative_to(ROOT)}")

    if args.print_text:
        for a, b, other_name, at in unstable:
            print(f"  [{other_name}] 第 {at} 字起：{a['text']}  /  {b['text']}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
