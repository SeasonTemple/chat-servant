"""量面板自己的撕边 —— 票 44 的纸条必须和它一致，否则读成黑框。

## 为什么先量这个

`stripgen.py` 那一版把撕口画成**一条 1px 深的密度线**，
用户当场指出「边缘得是和面板的那种一致啊？你这样和黑框有什么区别？」。

`ui/scripts/sheetprobe.ps1` 已经量过撕口深度，但它给的是**区间**，
没给**形状**（渐变？断续？参差？），也没给四条的差别。
要裁出真边，就得知道边在哪、往里凹多少。

## ⚠ 两条量具坑（这一版才踩到）

**坑一：alpha 阈值不能写死 255。** 实测这张 PNG **没有一个 A=255 的像素**，
满透明区是 255 而纸面最高只到 254 左右；于是 `A >= 255` 整列返回 -1，
而结果「-1 源px」看起来像「内缩 -0.5 CSS px」这种完全合理的数，
**不报错、不为空、纯错**。⇒ 阈值必须相对全图最大值取。

**坑二：控制台是 GBK，中文全变 mojibake。** 本项目的教训（见 AGENTS.md）：
脚本落文件再读，不要靠 stdout 看中文。

**坑三（2026-11-01 才发现，此前两版都错）：缩放常数不能取 380×560。**
本脚本原来写 `FRAME_W, FRAME_H = 380.0, 560.0`，即把 760×1120 源图当成铺满
整张面板。实际几何在 `ui/static/probe-sheet.svg` 里：

    <svg width="380" height="560" viewBox="0 0 380 560">
      <g filter="url(#sheetDrop)" transform="translate(16,0)">
        <image x="0" y="0" width="348" height="512" ...>

`.cs-sheet` 用 `background: url('/probe-sheet.svg') center / 100% 100%` 把这张
SVG 拉到 380×560，而 1 SVG 用户单位本来就等于 1 面板 CSS px ⇒
**源 px -> 面板 px 的正确系数是 348/760 = 0.4579（横）、512/1120 = 0.4571（纵），
且纸的左缘在面板 x=16。** 旧系数 380/760 = 0.5 偏大 1.09 倍，还会把 x 整体
左移 16px——于是「顶边内缩 12~27 源 px」被说成「6~13.5 CSS px」，而真值是
5.5~12.8（顶边）与 8.7~21.5（底边）。量具错了，引它的文档全跟着错。

⇒ 教训不是「这次算错了」，而是**量具自己也得有出处**：这条系数来自 SVG 的
`<image width/height>` 与 `<g transform>`，改素材时这两处会变，脚本必须从 SVG
里读，不能写死。下面 `svg_geometry()` 就是干这个的。

跑：uv run --with pillow python ui/scripts/_tearprobe.py
"""

from __future__ import annotations

from pathlib import Path

from PIL import Image
import sys

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

HERE = Path(__file__).resolve().parent.parent  # ui/
SRC = HERE / "static" / "probe-sheet.png"
SVG = HERE / "static" / "probe-sheet.svg"
OUT = Path("target/probe/tearprobe.txt")


def svg_geometry() -> tuple[float, float, float, float, float]:
    """从 SVG 读出 (画布宽, 画布高, 纸宽, 纸高, 纸左边距)，都是面板 CSS px。"""
    import re

    text = SVG.read_text(encoding="utf-8")
    canvas = re.search(r'<svg[^>]*width="([\d.]+)"[^>]*height="([\d.]+)"', text)
    image = re.search(r'<image[^>]*width="([\d.]+)"[^>]*height="([\d.]+)"', text)
    shift = re.search(r'transform="translate\(([-\d.]+),', text)
    if not (canvas and image and shift):
        raise SystemExit("probe-sheet.svg 结构变了：读不到 svg/image/translate —— 量具要改")
    return (
        float(canvas.group(1)),
        float(canvas.group(2)),
        float(image.group(1)),
        float(image.group(2)),
        float(shift.group(1)),
    )


def main() -> None:
    im = Image.open(SRC).convert("RGBA")
    w, h = im.size
    px = im.load()
    canvas_w, canvas_h, paper_w, paper_h, paper_x = svg_geometry()
    sx = paper_w / w          # 源 px -> 面板 CSS px（横向）
    sy = paper_h / h          # 源 px -> 面板 CSS px（纵向）
    # ⚠ 纸的左缘在面板 x=paper_x，所以「源图第 0 列」不等于「面板第 0 列」。

    amax = max(px[x, y][3] for y in range(0, h, 3) for x in range(0, w, 3))
    thr = amax - 6
    L: list[str] = []
    p = L.append

    p(f"源图 {w}x{h}")
    p(f"SVG 画布 {canvas_w:g}x{canvas_h:g}，纸 <image> {paper_w:g}x{paper_h:g} 左缘 x={paper_x:g}")
    p(f"源 px -> 面板 CSS px：x {sx:.4f}  y {sy:.4f}（{1 / sx:.2f} 源 px / CSS px）")
    p("⚠ 这两个数从 SVG 的 <image width/height> 与 <g transform> 读出来，不写死（坑三）")
    p(f"alpha 最大值 {amax}   实心阈值 A>={thr}（相对最大值，坑一）")
    p("")

    # ── 顶边 ──────────────────────────────────────────────────────
    p("── 顶边内缩（沿 x 找第一行实心），每 8 源 px ──")
    tops = []
    for x in range(0, w, 8):
        y = -1
        for yy in range(h):
            if px[x, yy][3] >= thr:
                y = yy
                break
        tops.append(y)
        p(f"  x={x:4}  内缩 {y:3} 源px = {y * sy:5.1f} CSS px")
    valid = [v for v in tops if v >= 0]
    p(f"  ⇒ 顶边内缩 min={min(valid)} max={max(valid)} "
      f"均值={sum(valid) / len(valid):.1f} 源px（= "
      f"{sum(valid) / len(valid) * sy:.1f} CSS px）")
    p("    ⚠ 这个 min/max 把**画幅两端的空隙列**也算进来了：x=0 与 x=744 那几列整条"
      "透明，读到的是 -1 与 208 这种数，它们不是撕口。下文另有「内容列」区间。")
    p("")

    # ── 顶边 / 底边：只取内容列（纸的实际横向范围，去掉两端收边） ──────
    # ⚠ 为什么单独一段：整幅 min/max 会被两端的空隙列与圆角收边支配（顶边读到
    #   208 源 px、底边两侧读到 43 CSS px），而那些列上面根本没有内容。
    #   布局关心的只有「内容会落在的那一段」，所以判据必须收在这里。
    p("── 顶边 / 底边内缩：只取内容列（面板 x 32 .. 348）──")
    # ⚠ 换算式必须**先减纸左缘**再除 sx：源列 0 对应面板 x=paper_x，不是 x=0。
    #   写成 32/sx 会得到源列 70，实际应为 (32-16)/sx = 35；右端同理少算 16px。
    band = range(int(round((32 - paper_x) / sx)), int(round((348 - paper_x) / sx)))
    for label, finder in (("顶边", "top"), ("底边", "bottom")):
        vals = []
        for x in band:
            if not (0 <= x < w):
                continue
            if finder == "top":
                y = next((yy for yy in range(h) if px[x, yy][3] >= thr), None)
                if y is not None:
                    vals.append(y)
            else:
                y = next((yy for yy in range(h - 1, -1, -1) if px[x, yy][3] >= thr), None)
                if y is not None:
                    vals.append(h - 1 - y)
        if not vals:
            p(f"  {label}：该区间内无实心像素")
            continue
        srt = sorted(vals)
        q = lambda pct: srt[min(len(srt) - 1, int(len(srt) * pct / 100))]
        p(f"  {label}内缩 {srt[0]}~{srt[-1]} 源px = {srt[0] * sy:.1f}~{srt[-1] * sy:.1f} "
          f"面板 CSS px   p50={q(50) * sy:.1f}  列数 {len(srt)}")
    p("")

    p("── 顶边 alpha 剖面（20 行，看渐变层数）──")
    for x in (120, 260, 400, 540, 660):
        col = []
        prev = -1
        for yy in range(0, 40):
            a = px[x, yy][3]
            if a == prev and len(col) >= 6 and a == 255:
                col.append("...")
                break
            col.append(a)
            prev = a
        p(f"  x={x:4}  {col}")
    p("")

    p("── 顶边深度的分布（内缩值 -> 出现次数，0/1/2/... 各多少列）──")
    hist: dict[int, int] = {}
    for x in range(0, w, 2):
        y = -1
        for yy in range(h):
            if px[x, yy][3] >= thr:
                y = yy
                break
        hist[y] = hist.get(y, 0) + 1
    for k in sorted(hist):
        p(f"  内缩 {k:3} 源px  ({k * sy:5.1f} CSS)  {hist[k]:4} 列"
          f"  {'#' * min(60, hist[k] // 6)}")
    p("")

    # ── 左 / 右 / 下 ──────────────────────────────────────────────
    p("── 左边内缩（沿 y），每 16 源 px ──")
    lefts = []
    for y in range(0, h, 16):
        x = -1
        for xx in range(w):
            if px[xx, y][3] >= thr:
                x = xx
                break
        lefts.append(x)
        p(f"  y={y:4}  内缩 {x:3} 源px = {x * sx:5.1f} CSS px")
    lv = [v for v in lefts if v >= 0]
    p(f"  ⇒ 左边内缩 min={min(lv)} max={max(lv)} 均值={sum(lv) / len(lv):.1f} 源px")
    p("")

    p("── 右 / 下边内缩 ──")
    for y in (0, 40, 200, 600, 1000, h - 1):
        x = w - 1
        while x >= 0 and px[x, y][3] < thr:
            x -= 1
        p(f"  y={y:4}  右内缩 {w - 1 - x:3} 源px = {(w - 1 - x) * sx:5.1f} CSS px")
    for x in (0, 40, 200, 400, 600, w - 1):
        y = h - 1
        while y >= 0 and px[x, y][3] < thr:
            y -= 1
        p(f"  x={x:4}  下内缩 {h - 1 - y:3} 源px = {(h - 1 - y) * sy:5.1f} CSS px")
    p("")

    p("── 四角：横向/纵向各多少源 px 才实心（决定能不能直接拿真角当九宫格）──")
    for name, (ax, ay, dx, dy) in {
        "左上": (0, 0, 1, 1),
        "右上": (w - 1, 0, -1, 1),
        "左下": (0, h - 1, 1, -1),
        "右下": (w - 1, h - 1, -1, -1),
    }.items():
        ix = next((k for k in range(40) if px[ax + dx * k, ay][3] >= thr), -1)
        iy = next((k for k in range(40) if px[ax, ay + dy * k][3] >= thr), -1)
        p(f"  {name}: 横向 {ix:3} 源px ({ix * sx:.1f} CSS)   "
          f"纵向 {iy:3} 源px ({iy * sy:.1f} CSS)")
    p("")

    p("── 中部纸芯取样（判断哪里是干净区，供 stripgen 用）──")
    for y in (400, 500, 600):
        row = [px[x, y][:3] for x in (300, 340, 380)]
        p(f"  y={y}  x=300/340/380 -> {row}")

    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_text("\n".join(L), encoding="utf-8")
    print(f"written: {OUT}")


if __name__ == "__main__":
    main()
