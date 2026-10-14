#!/usr/bin/env python3
"""应用图标生成器 —— 笔迹与纸的唯一来源，和 pencilgen.py 是同一条规矩。

设计定稿（实测 2026-10-13，判据是 16px 的托盘真实像素，不是大尺寸好看）：
    一张撕边纸（`ui/static/probe-sheet.png`）+ 一道横跨它的红勾。
    勾用 `src/lib/marks.js` 里的 `TICK_NESTED_BIG`，**不手画**。

    画布 512。纸 286（占 56%）居中；勾宽 350、重心略压低到 (256, 262)，
    两端恰好探出纸外 —— 这点是刻意的：全缩进纸里会退化成普通 app icon，
    两端探出才有「一笔横跨过去」的手感。16px 上读出来是「一块暖色的纸 +
    一个横跨它的红勾」，纸立得住、勾也立得住。

⚠ 纸的**调子**在 2026-10-14 被票 34 重做过：不再是压暗的 brightness 0.52，
   而是「奶白纸 + 深墨描边 1px + 实心深红勾」的双色分置方案。
   「撕边纸 + 一道横跨的红勾」这个设计本身没变，改的是**纸怎么被看见** ——
   用户在真机托盘上看到的是「一团灰」，票 29 记的代价已经兑现。
   理由、实测算式与被否掉的替代方案见下面「载体重做」一节。

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

from PIL import Image, ImageEnhance, ImageFilter

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
#   PAPER_SATURATE    纸的饱和度系数。1.0 = 现状。
#   PAPER_SHADOW_CSS  纸上的投影 CSS。票 29 原先建议的是「极淡的暖色暗边
#                     ／投影」，实测 1.11 → 1.12 等于零——投影在 16px 上是
#                     **亚像素**的（1024 开窗的 6px 缩到 16px 只剩 0.09px）。
#
# ⚠ 三个开关现在**都不是票 29 定的那个值**了，是票 34（2026-10-14）重做的定案。
#   下面「载体重做」一节写清楚了为什么，以及为什么连饱和度都放弃了。
PAPER_BRIGHTNESS = 1.0
PAPER_SATURATE = 1.0
PAPER_SHADOW_CSS = ""

# ── 载体重做（票 34，2026-10-14 定案）────────────────────────────────
#
# 为什么不能继续调亮度：这条**可证伪，不是没试**。`iconfeasible.py` 实算，
# 两种任务栏从两侧把纸的相对亮度夹死在 L ∈ [0.1411, 0.2654]（区间宽 0.1243），
# 而票 29 选的 0.52 落在 L=0.1950，离区间中点只差 **0.0083** —— 它已经在
# 最优处。任何单一灰度值都只有这一个解，两侧对比度此消彼长。
#
# ⇒ 解法不是找一个更好的灰度，而是**让两侧各由不同元素负责**：
#     · 浅色任务栏（L=0.896）靠**深**元素跳 → 深墨描边
#     · 深色任务栏（L=0.014）靠**亮**元素跳 → 奶白纸（brightness 1.0）
#   即「双色分置」。纸回到奶白，用户在托盘上看到的「一团灰」消失。
#
# `iconsilhouette.py` 按分侧门槛实测（深侧 median ≥ 8:1 要「跳出来」，
# 浅侧 readable ≥ 30% 要「不糊」），定案这一组：
#     浅侧 40% ✓ / 深侧 median 10.86 : 1 ✓
# 对比票 29 那张：深侧只有 3.80 : 1，正是用户抱怨「红勾反而最亮」。
#
# ⚠ 纸的饱和度**不做**（`PAPER_SATURATE` 留 1.0）。试过 1.3/1.6/2.0：
#   2.0 时 `ticksep.py` 直接 abort —— 纸的 R−B 差涨到 61，越过它判定「勾」的
#   门槛 60，纸被误判进勾的桶里。色相差被吃掉，勾会读不出来。
#
# ⚠ PAPER_STROKE_PX 是**零模糊**的 SVG `feMorphology` 膨胀（不是投影晕）。
# ⚠ **单位换算钉死在这里**，中间隔着两次缩放（实测推导）：
#     DESIGN 512 --×k(=2)--> 1024 画布 --÷64--> 托盘 16px
#   所以 16px 上 1px = 1024 上 64px = **DESIGN 坐标 32**。
#   32 = 托盘 16px 上 1px；64 = 2px（实测过粗，把纸挤小、深侧掉到 3.19）。
PAPER_STROKE_PX = 32
PAPER_STROKE_COLOR = "#3A2F1E"

# 勾改成实心深红。`TICK_NESTED_BIG` 是 5 条 opacity 0.22…0.58 的叠层，
# 半透明叠出来的层次在 16px 上读不出，只剩一个「偏粉」的合成色
# （票 29 记过这条欠账）。实心深红在浅色任务栏上是 7.5 : 1。
TICK_SOLID = True
TICK_SOLID_COLOR = "#8F2A1E"

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


def paper_png(scale):
    """纸图层：亮度 / 饱和度 / 描边**全在 PIL 里做完**，落成 PNG。

    为什么纸不交给浏览器
    --------------------
    描边最早写成 8 向 CSS `drop-shadow`（Chrome 挂死），改成 SVG
    `feMorphology` 后能出图，但代价是**每次渲染都让 1024² 的浏览器去跑
    像素级膨胀** —— 纯 CPU 光栅化，一个候选几秒，连渲十几个就把机器压住。
    本轮实测踩到：查进程列表的 `Get-CimInstance` 自己都超时 60 秒。

    ⇒ 而 alpha 膨胀是**纯图像学操作**，PIL 做得又快又不占外部资源：
        `MaxFilter(3)` 在每个像素取邻域最大值，迭代 r 次 == 半径 r 的膨胀。
    ⇒ 描边是**平涂**的、不是渐变，所以「先膨胀 alpha 再填墨色」与
      `feMorphology + feFlood` 的结果等价，不需要浏览器参与。

    ⇒ 浏览器只剩它不可替代的一件事：光栅化勾的贝塞尔 path。
       合成时它只贴一张现成 PNG，**不跑任何滤镜**。

    ⚠ **顺序错了等于没做**（本轮实际踩到，量具查到墨占比 15% → 只有勾、没有描边）：
    PIL 的 `alpha_composite` **不扩展画布**。所以「先把纸 resize 成整张画布、
    再在这张画布上膨胀」是错的——纸充满画布时，膨胀出的墨**必然溢出边界被裁掉**，
    成品上一点墨都没有。
    ⇒ 必须**先把纸放进一张预留好余量的画布**，膨胀才落在画布内。
    预留量 `pad = radius`（不是 radius+2）：墨从纸边向外正好到画布边界。

    ⚠ 画布留了余量之后，SVG `<image>` 的矩形**也要跟着外扩**同样的 pad，
      否则描边会被 `<image>` 的 width/height 裁掉（SVG 裁 image 内容）。
      忘了这一步的症状同样是 ink≈0，但成因完全不同，必须靠看中间产物区分。

    ⚠ 别为了省时间把 `RENDER` 调小：1024 是实测教训（见模块 docstring），
      1024 → 512 → 32 → 16 的缩放链必须保持，否则 master 的 alpha bbox
      会缩到角落，图标在 16px 上只剩 8×8 而 PNG 尺寸照样标称 16×16。

    返回 (padded_image, padding)：HTML 那边要把 image 的 x/y/width/height
    一起外扩同样的 padding，纸才落在原来的位置。
    """
    px = int(PAPER_BOX * scale)
    radius = int(round(PAPER_STROKE_PX * scale)) if PAPER_STROKE_PX > 0 else 0
    pad = radius
    size = px + 2 * pad

    paper = Image.open(PAPER).convert("RGBA").resize((px, px), Image.LANCZOS)
    if PAPER_BRIGHTNESS != 1.0 or PAPER_SATURATE != 1.0:
        r, g, b, a = paper.split()
        rgb = Image.merge("RGB", (r, g, b))
        if PAPER_BRIGHTNESS != 1.0:
            rgb = ImageEnhance.Brightness(rgb).enhance(PAPER_BRIGHTNESS)
        if PAPER_SATURATE != 1.0:
            rgb = ImageEnhance.Color(rgb).enhance(PAPER_SATURATE)
        paper = Image.merge("RGBA", (*rgb.split(), a))

    canvas = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    canvas.alpha_composite(paper, (pad, pad))

    if radius > 0:
        # MaxFilter 的核只能是奇数，3×3 迭代 r 次 == 半径 r 的膨胀。
        # 1024 画布上 radius=64 要迭代 64 遍，实测 PIL 的 C 实现很快——
        # 整个渲染（含 Chrome 光栅化勾）只要 3 秒，而同样的活交给浏览器
        # 滤镜链时既慢又会挂死。
        alpha = canvas.getchannel("A")
        for _ in range(radius):
            alpha = alpha.filter(ImageFilter.MaxFilter(3))
        ink = Image.new("RGBA", (size, size), PAPER_STROKE_COLOR)
        ink.putalpha(alpha)
        canvas = Image.alpha_composite(ink, canvas)
    return canvas, pad


def build_html(scale, paper_path, pad):
    items = tick_paths()
    x0, y0, x1, y1 = bbox(items)
    # scale 已经是「开窗尺寸 / 设计尺寸」的倍数，别再除一次 DESIGN
    k = scale
    if TICK_SOLID:
        # 5 条叠层压成一条实心：16px 上层次读不出，只剩一个合成色
        paths = "".join(f'<path d="{d}" fill="{TICK_SOLID_COLOR}"/>' for d, _ in items)
    else:
        paths = "".join(
            f'<path d="{d}" fill="{RED}" fill-opacity="{o}"/>' for d, o in items
        )
    tick_w = TICK_W * k
    tick_h = TICK_W * k * (y1 - y0) / (x1 - x0)
    tx = (TICK_CX * k) - tick_w / 2
    ty = (TICK_CY * k) - tick_h / 2
    paper = PAPER_BOX * k
    pl = (DESIGN * k - paper) / 2
    # image 的矩形要跟着外扩 pad，纸才落在原位；不外扩的话描边被裁掉
    ix, iy = pl - pad, pl - pad
    iw = ih = paper + 2 * pad
    return f"""<!doctype html>
<html><head><meta charset="utf-8"><style>
  html,body {{ margin:0; padding:0; background:transparent; }}
  .icon {{ position:relative; width:{DESIGN * k:.0f}px; height:{DESIGN * k:.0f}px; }}
  svg {{ display:block; }}
</style></head><body><div class="icon">
  <svg width="{DESIGN * k:.0f}" height="{DESIGN * k:.0f}"
       viewBox="0 0 {DESIGN * k:.0f} {DESIGN * k:.0f}">
    <image href="file:///{paper_path.as_posix()}"
           x="{ix:.1f}" y="{iy:.1f}" width="{iw:.1f}" height="{ih:.1f}"
           preserveAspectRatio="none"/>
    <g transform="translate({tx:.2f},{ty:.2f}) scale({tick_w / (x1 - x0):.5f})
       translate({-x0:.2f},{-y0:.2f})">{paths}</g>
  </svg>
</div></body></html>"""


def shoot(html_path, png_path, size):
    """一次渲染 = 一个**全新的** profile，用完立刻删。

    ⚠ 这里的坑是本轮第四次「静默失效」，而且最难查：
      原来的 profile 名是 `hash(html_path)`，而 html_path 是**固定路径**，
      于是每一次渲染都复用同一个 profile 目录 —— Chrome 对 `file://` 的
      缓存也就一起复用了。后果是 `icon-paper.png` 明明已经重新生成、
      HTML 里的 image 矩形也已经改对，**出图却还是缓存里那版**，
      全程不报任何错。表现是「改了参数但数字一动不动」，
      和「参数根本没写对」在外观上完全一样。
    ⇒ 改成 `mkdtemp` 一次性 profile，`finally` 里删干净：
      既不命中缓存，也不留垃圾目录（TEMP 下曾积过多个 icongen-*）。
    """
    import shutil
    import tempfile

    profile = tempfile.mkdtemp(prefix="icongen-")
    cmd = [
        CHROME, "--headless=new", f"--user-data-dir={profile}", "--no-first-run",
        "--disable-gpu", "--hide-scrollbars", "--default-background-color=00000000",
        f"--window-size={size},{size}", "--virtual-time-budget=4000",
        f"--screenshot={png_path}", f"file:///{html_path}",
    ]
    try:
        subprocess.run(cmd, capture_output=True, timeout=120)
        if not os.path.exists(png_path):
            raise SystemExit(f"ABORT: chrome produced nothing ({size}px)")
    except subprocess.TimeoutExpired:
        # 没有超时的话，Chrome 在某些滤镜组合下会**永久挂住不退**，
        # 脚本就跟着卡死，表现成「渲染中」一直不出图。宁可报错也不要静默等。
        raise SystemExit(f"ABORT: chrome did not exit within 120s ({size}px) "
                         f"- a filter combination is hanging the renderer")
    finally:
        shutil.rmtree(profile, ignore_errors=True)


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
    # 纸先在 PIL 里调好并落成 PNG，浏览器只负责光栅化勾的 path
    paper = WORK / "icon-paper.png"
    paper_img, pad = paper_png(scale)
    paper_img.save(paper)
    html = WORK / "icon-master.html"
    io.open(html, "w", encoding="utf-8").write(build_html(scale, paper, pad))
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
    ap.add_argument("--saturate", type=float, default=PAPER_SATURATE)
    ap.add_argument("--shadow", default=PAPER_SHADOW_CSS)
    ap.add_argument("--stroke", type=float, default=PAPER_STROKE_PX,
                    help="纸的硬描边粗细，单位是 512 设计坐标："
                         "32 = 托盘 16px 上 1px，64 = 2px（0 = 不描边）")
    ap.add_argument("--tick-solid", action="store_true", default=TICK_SOLID,
                    help="勾压成实心深红（5 条半透明叠层在 16px 上只剩偏粉合成色）")
    a = ap.parse_args()
    PAPER_BRIGHTNESS = a.brightness
    PAPER_SATURATE = a.saturate
    PAPER_SHADOW_CSS = a.shadow
    PAPER_STROKE_PX = a.stroke
    TICK_SOLID = a.tick_solid
    main(a.out)