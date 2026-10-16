"""量真实微信帧上的**渲染字号**：用同一把尺子，在已知字号下标定。

## 为什么值得量

`ocrbench` 已经量出字符错误率随字号变化极大（2× 时 12px 档 CER 0.41，
21px 档 0.067，差六倍）。所以「微信到底把字渲染成几 px」不是审美问题，
它**直接决定**该选几倍放大、以及真实聊天里的准确率落在哪一档。

## 判据：每行的**边缘密度**

气泡内部是平色 ⇒ 那一行除了气泡左右两条边之外没有任何亮度跳变；
字有笔画 ⇒ 一行里几十个跳变。于是「这一行有没有字」不需要先把气泡切出来。

⚠ 前面两次都量错了，两次都记在这儿：

1. **第一版用「与行中位亮度的差」当墨迹。** 气泡填充色（灰 74）与聊天区背景
   （暗）的差也过了阈值 ⇒ 量到的是**气泡高度**（含 padding），不是字高；
   而标定那张图的裁剪框开得比图像还大，Pillow 把框外补成 0，行中位数变成 0，
   整行都被算成墨迹 ⇒ 标定档恰好等于 `size * 3`，那是画布高度。
2. **第二版试图先切气泡再在内部量。** 阈值 `MIN_BUBBLE_PX=200` 比微信的气泡
   还宽 ⇒ 一个都没切出来。切气泡本身又依赖「背景色」这个假设，而聊天区里
   同时有对方灰气泡、自己绿气泡、图片消息和头像，背景色并不单一。

⇒ 第三版换成边缘密度，**不依赖背景色假设**，也不用切气泡。

⚠ 两边必须用完全相同的量法。口径差一点点就会落到相邻档位——
   而相邻档位在这个测量里正是要区分的东西。
"""

from __future__ import annotations

import sys
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

ROOT = Path(__file__).resolve().parents[2]
FONT = Path(r"C:\Windows\Fonts\msyh.ttc")
SIZES = [12, 14, 16, 19, 21]

BUBBLE_BG = (74, 74, 74)      # 标定用的底色（对方灰气泡）
TEXT_ON_BUBBLE = (240, 240, 240)

EDGE_DELTA = 40    # 相邻像素亮度差 ≥ 这个值算「一条边」
MIN_EDGES = 8      # 一行至少这么多条边才算「这行有字」
MIN_BAND = 3       # 连续行段至少这么高才是一个文字带


def row_edges(crop: Image.Image) -> list[int]:
    """每一行的边缘数。"""
    g = crop.convert("L")
    w, h = g.size
    px = g.load()
    out: list[int] = []
    for y in range(h):
        n = 0
        prev = px[0, y]
        for x in range(1, w):
            v = px[x, y]
            if abs(v - prev) >= EDGE_DELTA:
                n += 1
            prev = v
        out.append(n)
    return out


def text_bands(crop: Image.Image) -> list[int]:
    """文字带的高度列表。"""
    edges = row_edges(crop)
    heights: list[int] = []
    start: int | None = None
    for i in range(len(edges) + 1):
        on = i < len(edges) and edges[i] >= MIN_EDGES
        if on and start is None:
            start = i
        elif not on and start is not None:
            heights.append(i - start)
            start = None
    return [h for h in heights if h >= MIN_BAND]


def median(xs: list[int]) -> int:
    s = sorted(xs)
    return s[len(s) // 2] if s else 0


def render_reference(text: str, size: int) -> Image.Image:
    font = ImageFont.truetype(str(FONT), size)
    w = max(80, int(len(text) * size * 1.8))
    h = size * 3
    img = Image.new("RGB", (w, h), BUBBLE_BG)
    ImageDraw.Draw(img).text((10, h // 2), text, font=font, fill=TEXT_ON_BUBBLE, anchor="lm")
    return img


def main() -> int:
    out: list[str] = []

    def emit(s: str = "") -> None:
        out.append(s)

    if len(sys.argv) < 3:
        emit("用法: measure_real_frame_font.py <帧.png> <left,top,right,bottom> [标定串]")
        return 2
    frame = Path(sys.argv[1])
    if not frame.is_absolute():
        frame = ROOT / frame
    left, top, right, bottom = (int(v) for v in sys.argv[2].split(","))

    img = Image.open(frame).convert("RGB")
    emit(f"帧 {img.width}x{img.height}  聊天区 = ({left},{top})-({right},{bottom})")

    # 逐 200px 宽的竖条切，避免把两条消息并进同一个带（并进去高度不变，但会连成一片）
    strip_w = 200
    bands: list[int] = []
    for x0 in range(left, right - strip_w, strip_w):
        bands += text_bands(img.crop((x0, top, x0 + strip_w, bottom)))
    bands.sort()
    emit(f"\n聊天区文字带 {len(bands)} 条: {bands}")
    if not bands:
        emit("一条都没量到——阈值或区域不对，别硬猜。")
    else:
        emit(f"中位 {median(bands)}  众数附近 {bands[len(bands) // 2]}")

    probe = sys.argv[3] if len(sys.argv) > 3 else "发来看看"
    emit(f"\n标定（{FONT.name}，串「{probe}」，同一量法）:")
    ref: dict[int, int] = {}
    for size in SIZES:
        r = render_reference(probe, size)
        hs = text_bands(r)
        ref[size] = median(hs)
        emit(f"  {size:>2}px → 带高 {hs} 中位 {ref[size]}")

    emit("\n判定:")
    if bands:
        h = median(bands)
        best = min(SIZES, key=lambda s: abs(h - ref[s]))
        emit(f"  实测墨迹高 {h}px  →  最接近 **{best}px**")
        emit("  各档差: " + "  ".join(f"{s}px:{h - ref[s]:+d}" for s in SIZES))
    emit(f"\n（阈值 EDGE_DELTA={EDGE_DELTA} MIN_EDGES={MIN_EDGES} 两边共用）")

    dest = ROOT / "target" / "probe" / "ocrbench" / "real-font-report.txt"
    dest.parent.mkdir(parents=True, exist_ok=True)
    dest.write_text("\n".join(out), encoding="utf-8")
    print("\n".join(out).encode("ascii", "replace").decode("ascii"))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
