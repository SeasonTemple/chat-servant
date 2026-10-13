#!/usr/bin/env python3
"""应用图标生成器 —— 笔迹与纸的唯一来源，和 pencilgen.py 是同一条规矩。

设计定稿（实测 2026-10-13，判据是 16px 的托盘真实像素，不是大尺寸好看）：
    一张撕边纸（`ui/static/probe-sheet.png`）+ 一道横跨它的红勾。
    勾用 `src/lib/marks.js` 里的 `TICK_NESTED_BIG`，**不手画**。

    画布 512。纸 286（占 56%）居中；勾宽 350、重心略压低到 (256, 262)，
    两端恰好探出纸外 —— 这点是刻意的：全缩进纸里会退化成普通 app icon，
    两端探出才有「一笔横跨过去」的手感。16px 上读出来是「一块暖色的纸 +
    一个横跨它的红勾」，纸立得住、勾也立得住。

    扫过的比例（`target/probe/judge-*.png` 留有对照）：
        纸 226 / 勾 400 → 勾两臂横甩出纸外，剪影变宽，读着散
        纸 286 / 勾 350 → **本稿**
        纸 286 / 勾 400 → 两臂甩得更远，更散
        纸 348 / 勾 330 → 干净，但退化成圆角方块，没性格

⚠ **出图必须按 1024 开窗，不能拿更小的图放大。**
早先一版把 `.icon`（512）放进 1024×1024 的视口，master 的 alpha bbox
实测是 (83,85,512,424) —— 图标只占了左上角一格，再 resize 到 16px
就只剩 8×8 像素。据此得出的「纸在 16px 上会消失」是个**假结论**，
差点把纸一路缩小。这里自带自检，见 `assert_fills_canvas`。

用法：
    python ui/scripts/icongen.py
"""

import io
import os
import re
import subprocess
import sys
from pathlib import Path

from PIL import Image

ROOT = Path(__file__).resolve().parents[2]
OUT = ROOT / "ui" / "src-tauri" / "icons"
WORK = ROOT / "target" / "probe"

MARKS = ROOT / "ui" / "src" / "lib" / "marks.js"
PAPER = ROOT / "ui" / "static" / "probe-sheet.png"

CHROME = r"C:\Program Files\Google\Chrome\Application\chrome.exe"

RED = "#c0392b"
DESIGN = 512          # 设计坐标系；所有比例都以它为基准
PAPER_BOX = 286       # 纸边长（512 坐标系）
TICK_W = 350          # 勾宽
TICK_CX, TICK_CY = 256, 262
RENDER = 1024         # 实际开窗尺寸，必须是 DESIGN 的整数倍
MASTER = 512          # 落盘的基准图

# ── 分离度实验口（票 29 未做完的那一项）─────────────────────────────
#
# 浅色任务栏 `#F3F3F3` 上，纸 `rgb(246,231,191)` 与背景只有 **1.11 : 1**
# （`iconsep.py` 实测），16px 上糊成一团淡斑；深色 `#1F1F1F` 上是 13.39:1。
# 两个待验方案，默认都是「什么都不改」：
#
#   PAPER_BRIGHTNESS  纸的亮度系数。1.0 = 现状。
#                     ⚠ 要在浅色背景上够到 3:1 得压到 **0.60**，那已经是
#                       一张深卡其色的纸，不再是「奶白纸」了。
#   PAPER_SHADOW_CSS  纸上的投影 CSS。票 29 原先建议的是「极淡的暖色暗边
#                     ／投影」，而投影在 16px 上是**亚像素**的——1024 开窗的
#                     3px 缩到 16px 只剩 0.05px，量出来是否等于零**要测**，
#                     不要靠推理下结论。
PAPER_BRIGHTNESS = 1.0
PAPER_SHADOW_CSS = ""

PNG_SIZES = {
    "32x32.png": 32,          # ⚠ tray_icon 用 include_bytes! 读这一张
    "64x64.png": 64,
    "128x128.png": 128,
    "128x128@2x.png": 256,
    "icon.png": 512,
}
ICO_SIZES = [(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)]


def tick_paths():
    """从 marks.js 取真实笔迹 path，不在这里重画勾。"""
    src = io.open(MARKS, encoding="utf-8").read()
    m = re.search(r"export const TICK_NESTED_BIG = \[(.*?)\n\];", src, re.S)
    if not m:
        raise SystemExit(f"ABORT: TICK_NESTED_BIG not found in {MARKS}")
    items = [(d, float(o)) for d, o in re.findall(r"\{ d: '([^']+)', o: ([\d.]+) \}", m.group(1))]
    if not items:
        raise SystemExit("ABORT: TICK_NESTED_BIG is empty")
    return items


def bbox(items):
    xs, ys = [], []
    for d, _ in items:
        vals = [float(v) for v in re.findall(r"-?\d+(?:\.\d+)?", d)]
        xs += vals[0::2]
        ys += vals[1::2]
    return min(xs), min(ys), max(xs), max(ys)


def build_html(scale):
    items = tick_paths()
    x0, y0, x1, y1 = bbox(items)
    # scale 已经是「开窗尺寸 / 设计尺寸」的倍数，别再除一次 DESIGN
    k = scale
    paths = "".join(
        f'<path d="{d}" fill="{RED}" fill-opacity="{o}"/>' for d, o in items
    )
    tick_w = TICK_W * k
    tick_h = TICK_W * k * (y1 - y0) / (x1 - x0)
    tx = (TICK_CX * k) - tick_w / 2
    ty = (TICK_CY * k) - tick_h / 2
    paper = PAPER_BOX * k
    pl = (DESIGN * k - paper) / 2
    # `filter` 里 brightness 作用在纸上（含撕口的 alpha，撕边不会被啃掉），
    # drop-shadow 作用在**纸的渲染结果**上，所以跟着撕边走，而不是一个方框晕。
    bits = []
    if PAPER_BRIGHTNESS != 1.0:
        bits.append(f"brightness({PAPER_BRIGHTNESS})")
    if PAPER_SHADOW_CSS:
        bits.append(f"drop-shadow({PAPER_SHADOW_CSS})")
    paper_filter = " ".join(bits) if bits else "none"
    return f"""<!doctype html>
<html><head><meta charset="utf-8"><style>
  html,body {{ margin:0; padding:0; background:transparent; }}
  .icon {{ position:relative; width:{DESIGN * k:.0f}px; height:{DESIGN * k:.0f}px; }}
  .paper {{ position:absolute; left:{pl:.1f}px; top:{pl:.1f}px;
            width:{paper:.1f}px; height:{paper:.1f}px;
            background:url('file:///{PAPER.as_posix()}') center/100% 100% no-repeat;
            filter:{paper_filter}; }}
</style></head><body><div class="icon">
  <div class="paper"></div>
  <svg style="position:absolute;inset:0" width="{DESIGN * k:.0f}" height="{DESIGN * k:.0f}"
       viewBox="0 0 {DESIGN * k:.0f} {DESIGN * k:.0f}">
    <g transform="translate({tx:.2f},{ty:.2f}) scale({tick_w / (x1 - x0):.5f})
       translate({-x0:.2f},{-y0:.2f})">{paths}</g>
  </svg>
</div></body></html>"""


def shoot(html_path, png_path, size):
    profile = os.path.join(os.environ["TEMP"], "icongen-" + str(abs(hash(html_path))))
    cmd = [
        CHROME, "--headless=new", f"--user-data-dir={profile}", "--no-first-run",
        "--disable-gpu", "--hide-scrollbars", "--default-background-color=00000000",
        f"--window-size={size},{size}", "--virtual-time-budget=4000",
        f"--screenshot={png_path}", f"file:///{html_path}",
    ]
    subprocess.run(cmd, capture_output=True)
    if not os.path.exists(png_path):
        raise SystemExit(f"ABORT: chrome produced nothing ({size}px)")


def assert_fills_canvas(im, size):
    """量具自检：图标必须撑满画布。

    这条是被一次真实翻车逼出来的 —— 拿小视口渲出来的图再缩小，
    内容会缩在角落，而截图尺寸**照样是标称值**，肉眼看不出来。
    """
    box = im.getbbox()
    if box is None:
        raise SystemExit("ABORT: fully transparent render")
    fill = ((box[2] - box[0]) * (box[3] - box[1])) / (size * size)
    if fill < 0.25:
        raise SystemExit(f"ABORT: content fills only {fill:.0%} of canvas, bbox={box}")
    return box


def main(out_dir=None):
    """`out_dir` 给 None 时落进 `ui/src-tauri/icons`，给路径时落进那里。

    分流是为了**实验**：分离度那项要渲好几个候选（亮度/投影），
    而候选不能盖掉托盘正在用的那张真图标。
    """
    out = Path(out_dir) if out_dir else OUT
    for p in (CHROME, str(MARKS), str(PAPER)):
        if not os.path.exists(p):
            raise SystemExit(f"ABORT: missing {p}")
    WORK.mkdir(parents=True, exist_ok=True)
    out.mkdir(parents=True, exist_ok=True)

    scale = RENDER / DESIGN
    html = WORK / "icon-master.html"
    io.open(html, "w", encoding="utf-8").write(build_html(scale))
    raw = WORK / "icon-master-1024.png"
    shoot(str(html), str(raw), RENDER)

    master = Image.open(raw).convert("RGBA")
    assert_fills_canvas(master, RENDER)
    if master.size != (RENDER, RENDER):
        raise SystemExit(f"ABORT: canvas is {master.size}, expected {RENDER}")
    master = master.resize((MASTER, MASTER), Image.LANCZOS)

    for name, size in PNG_SIZES.items():
        im = master.resize((size, size), Image.LANCZOS)
        im.save(out / name)
        print(f"  {name:<18} {size}x{size}  bbox={im.getbbox()}")
    print(f"  {'icon.ico':<18} {ICO_SIZES}")

    master.save(out / "icon.ico", format="ICO", sizes=ICO_SIZES)
    print(f"done -> {out}")


if __name__ == "__main__":
    import argparse

    ap = argparse.ArgumentParser()
    ap.add_argument("--out", default=None, help="输出目录（默认 ui/src-tauri/icons）")
    ap.add_argument("--brightness", type=float, default=PAPER_BRIGHTNESS)
    ap.add_argument("--shadow", default=PAPER_SHADOW_CSS)
    a = ap.parse_args()
    PAPER_BRIGHTNESS = a.brightness
    PAPER_SHADOW_CSS = a.shadow
    main(a.out)