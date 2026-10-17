"""把 `strip.png` 与面板的撕口放大到同一倍率并排比 —— 票 44。

## 为什么要比

用户截图指出：「边缘得是和面板的那种一致啊？」
「一致」有两层，量具必须分开量：

1. **形状**：参差的起伏节奏（这个 `stripgen.py` 已经自检过）
2. **质感**：撕口那一带的**明暗**与纸芯的对比
   —— 第一版画成 1px 深线时它是「均匀的深线」，
     这一版裁的是真撕口，但**没量过它比底纸暗多少**

⚠ 「看起来像」不算。撕口在纸上的读数靠的是**明暗差**，
而放大后很容易「看着挺像」，实际比底纸还淡。

本脚本把面板顶边的一段与 strip.png 的顶边一段
各自 NEAREST 放大到同一倍率（不用插值——插值会把参差抹平，
抹平之后的参差是假的），并排输出。

跑：uv run --with pillow python ui/scripts/_stripcompare.py
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
STRIP = HERE / "static" / "strip.png"
OUT = Path("target/probe/strip-compare.png")

ZOOM = 4
# 取面板顶边中段 128 源 px（= 64 CSS px），与 strip.png 的边同宽
X0, W = 316, 128

L: list[str] = []
p = L.append


def main() -> None:
    paper = Image.open(SRC).convert("RGBA")
    strip = Image.open(STRIP).convert("RGBA")

    # 面板顶边：y 0..40，含撕口（实测 y=16 起、y=19 变实心）
    a = paper.crop((X0, 0, X0 + W, 40))
    # strip 顶边：0..40 同理
    b = strip.crop((0, 0, strip.width, 40))

    # 放大到同宽后上下拼接
    # ⚠ `Image.resize` **返回的是 Image，不是元组**。
    #   写成 `aw, ah = a.resize(...)` 会报
    #   `TypeError: cannot unpack non-iterable Image object`
    #   —— 这类错比「图出不来」更难查，因为它看起来像 resize 坏了，
    #   实际是**拿返回值的方式**错了。
    aw_img = a.resize((a.width * ZOOM, a.height * ZOOM), Image.NEAREST)
    bw_img = b.resize((b.width * ZOOM, b.height * ZOOM), Image.NEAREST)
    aw, ah = aw_img.width, aw_img.height
    bw, bh = bw_img.width, bw_img.height
    gap = 12
    canvas = Image.new("RGBA", (aw, ah + gap + bh), (40, 40, 40, 255))
    canvas.paste(aw_img, (0, 0))
    canvas.paste(bw_img, (0, ah + gap))
    OUT.parent.mkdir(parents=True, exist_ok=True)
    canvas.save(OUT)

    # ── 量明暗：撕口那一带的亮度 vs 实心纸的亮度 ──────────────────
    def lum(c: tuple[int, int, int, int]) -> float:
        return 0.299 * c[0] + 0.587 * c[1] + 0.114 * c[2]

    def report(tag: str, im: Image.Image, base_y: int) -> None:
        px = im.load()
        ramp, solid = [], []
        for x in range(0, im.width, 2):
            # 沿这一列取「从边沿到实心」的所有半透明像素
            for k in range(0, base_y):
                c = px[x, k]
                if 10 < c[3] < 245:
                    ramp.append(lum(c))
                    break
            # 实心纸：取该图中心附近的像素
            for k in range(base_y // 2, base_y):
                c = px[x, k]
                if c[3] >= 250:
                    solid.append(lum(c))
                    break
        if not ramp or not solid:
            p(f"{tag}: ramp={len(ramp)} solid={len(solid)}  ← 有一侧是空的")
            return
        rl = sum(ramp) / len(ramp)
        sl = sum(solid) / len(solid)
        p(f"{tag}:")
        p(f"  半透明撕口像素 均亮度 {rl:.1f}  (n={len(ramp)})")
        p(f"  实心纸       均亮度 {sl:.1f}  (n={len(solid)})")
        p(f"  明暗差       {sl - rl:+.1f} / 255")

    p(f"放大倍率 NEAREST ×{ZOOM}")
    p(f"上图 = 面板顶边（{SRC.name}）   下图 = strip.png 顶边")
    p("")
    report("面板顶边", a, 40)
    report("strip 顶边", b, 40)
    p("")
    p(f"written: {OUT}")

    # ⚠ **中文必须落文件**：Windows 控制台是 GBK，print 出去全是 mojibake，
    #   而这份报告里全是中文标签 —— stdout 上根本没法读。
    #   本项目的规矩（AGENTS.md 也记着）：脚本落文件再读。
    out_txt = OUT.with_suffix(".txt")
    out_txt.write_text("\n".join(L), encoding="utf-8")
    print(f"written: {out_txt}")


if __name__ == "__main__":
    main()
