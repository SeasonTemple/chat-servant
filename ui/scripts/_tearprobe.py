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

跑：uv run --with pillow python ui/scripts/_tearprobe.py
"""

from __future__ import annotations

from pathlib import Path

from PIL import Image

HERE = Path(__file__).resolve().parent.parent  # ui/
SRC = HERE / "static" / "probe-sheet.png"
OUT = Path("target/probe/tearprobe.txt")

FRAME_W, FRAME_H = 380.0, 560.0


def main() -> None:
    im = Image.open(SRC).convert("RGBA")
    w, h = im.size
    px = im.load()
    sx = FRAME_W / w
    sy = FRAME_H / h

    amax = max(px[x, y][3] for y in range(0, h, 3) for x in range(0, w, 3))
    thr = amax - 6
    L: list[str] = []
    p = L.append

    p(f"源图 {w}x{h}   面板 CSS 缩放 x{sx:.4f} y{sy:.4f}")
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
