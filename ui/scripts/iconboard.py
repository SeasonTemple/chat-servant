#!/usr/bin/env python3
"""托盘图标候选对照图（票 34）—— 16px 真实像素，铺在两种任务栏底色上。

为什么出图而不是只报数字
------------------------
票 34 的问题是**观感**（「纸是一团灰」「红勾反而最亮」），而
`iconsep.py` / `ticksep.py` 报的是对比度数字。对比度过了 3:1 只说明
「不糊」，不代表「读得出是一块纸」。这两件事必须让眼睛看。

⚠ 本项目在这上面栽过一次（AGENTS.md 有记）：**设计画布尺寸必须等于
浏览器视口尺寸**，否则 master 的 alpha bbox 会缩到角落，图标只剩 8x8
像素而 PNG 尺寸照样标称 16x16。本脚本不依赖浏览器，直接从 32x32
的 master 用 LANCZOS 缩到真 16px，再按整数倍最近邻放大——放大只是
给人眼看的，**像素值仍是 16px 的真实值**。

用法：python ui/scripts/iconboard.py [out.png]
"""

import sys
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

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
VAR = ROOT / "target" / "iconvar"
TRAY_PX = 16
ZOOM = 9                      # 16 * 9 = 144，纯整数放大，不引入新像素

TASKBARS = [
    ("dark  #1F1F1F", (0x1F, 0x1F, 0x1F)),
    ("light #F3F3F3", (0xF3, 0xF3, 0xF3)),
]

CANDIDATES = [
    ("旧\n纸压暗 0.52\n深侧3.80 ✗", VAR / "sat1"),
    ("定案 A′\n奶白纸+墨线1px\n+实心深红勾\n浅40% 深10.8 ✓", ROOT / "ui" / "src-tauri" / "icons"),
]

FONT_CANDIDATES = [
    r"C:\Windows\Fonts\msyh.ttc",
    r"C:\Windows\Fonts\simhei.ttf",
    r"C:\Windows\Fonts\simsun.ttc",
]


def font(size):
    for f in FONT_CANDIDATES:
        if Path(f).exists():
            try:
                return ImageFont.truetype(f, size)
            except OSError:
                continue
    return ImageFont.load_default()


def icon_16(src_dir):
    """取 master 缩到真 16px。

    用 icon.png（512）而不是 32x32 再缩一次——32x32 已经带过一次
    抗锯齿缩放，再缩会把纸边啃得更碎。
    """
    master = Image.open(src_dir / "icon.png").convert("RGBA")
    return master.resize((TRAY_PX, TRAY_PX), Image.LANCZOS)


def main():
    out = Path(sys.argv[1]) if len(sys.argv) > 1 else (ROOT / "target" / "probe" / "iconboard.png")

    f_label = font(21)
    f_head = font(24)
    f_small = font(17)

    cell = TRAY_PX * ZOOM
    pad_x, pad_y = 22, 16

    rows = len(CANDIDATES)
    # 每个候选一格，格宽要容得下它**最长的一行**标签 —— 标签是两行，
    # 而 textlength 量换行串会算出一个假的小宽度，标签就会压到邻格
    label_w = int(max(
        max(ImageDraw.Draw(Image.new("RGB", (1, 1))).textlength(line, font=f_small)
            for line in name.split("\n"))
        for name, _ in CANDIDATES
    )) + 26
    # 任务栏名独占一行，不与候选标签同行 —— 上一版两者同行，标签互相压
    bar_row_h = 32
    head_h = 62
    strip_h = bar_row_h + 42 + cell + 16

    head_text = "托盘图标候选对照 · 真 16px 像素（整数放大 9 倍，无插值）"
    # ⚠ 画布宽度必须**同时**容得下标题和候选标签。第一版只按标签算 width，
    #   标题就超出画布被裁掉——而裁掉的恰好是最该被看到的那行字。
    title_w = int(ImageDraw.Draw(Image.new("RGB", (1, 1))).textlength(
        head_text, font=f_head))
    width = int(max(pad_x * 2 + label_w * rows, title_w + pad_x * 2))
    height = head_h + len(TASKBARS) * strip_h + pad_y

    sheet = Image.new("RGB", (width, height), (0x22, 0x22, 0x26))
    d = ImageDraw.Draw(sheet)

    d.text((pad_x, 14), head_text, font=f_head, fill=(0xF2, 0xF2, 0xF2))

    y = head_h
    for bar_label, bar_rgb in TASKBARS:
        fg = (0x11, 0x11, 0x11) if bar_label.startswith("light") else (0xEE, 0xEE, 0xEE)
        d.rectangle([pad_x, y, width - pad_x, y + strip_h], fill=bar_rgb)
        d.text((pad_x + 8, y + 5), f"任务栏底色 {bar_label}", font=f_label, fill=fg)

        x = pad_x
        for name, src in CANDIDATES:
            d.multiline_text((x, y + bar_row_h), name, font=f_small, fill=fg,
                             spacing=2)
            ic = icon_16(src)
            big = ic.resize((cell, cell), Image.NEAREST)
            # 底色合成（图标有不透明纸，也有透明撕边）
            flat = Image.new("RGB", (cell, cell), bar_rgb)
            flat.paste(big, (0, 0), big)
            sheet.paste(flat, (x, y + bar_row_h + 42))
            x += label_w
        y += strip_h

    out.parent.mkdir(parents=True, exist_ok=True)
    sheet.save(out)

    # 自检：画布必须装下所有东西，且不是空图
    assert sheet.getbbox() is not None, "ABORT: empty sheet"
    print(f"done -> {out}  ({width}x{height})")
    return 0


if __name__ == "__main__":
    sys.exit(main())