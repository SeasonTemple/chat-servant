#!/usr/bin/env python3
"""量应用图标在两种任务栏背景上的分离度。

为什么需要它
------------
托盘图标所在的背景不是恒定的：Windows 浅色主题的任务栏是接近白的
`#F3F3F3`，深色主题是接近黑的 `#1F1F1F`。图标里的奶白纸 `#EFE6CE`
在**浅色**那一侧几乎与背景同值，16px 上糊成一团淡斑；在深色那一侧
则很清楚。

票 29 记的是「看起来融掉」，而**看起来融掉**与「量出来确实只有
1.3:1」在 16px 的托盘上是同一件事。所以这里量一个**有区分度**的量。

⚠⚠ **判据是对比度，不是亮度差。**
第一版这里量的是 `|L(像素) − L(背景)|` 的 p95，而那张表给出的
`p95 ΔL = 0.49` 看着像「分离得很好」，同一份数据算出的对比度却是
**1.28 : 1** —— 因为两个颜色都落在 L≈0.9 的近白区，绝对差 0.49
相对于 0.9 根本不算什么。
⇒ **绝对亮度差在近白/近黑两端会骗人**，只有相对对比度能跨量级比较。
这正是「指标名与它量到的东西对不上 ⇒ 结论整个反了」那条。

量的东西
--------
1. `纸` = 不透明像素里的**众数颜色**（也就是纸本身），量它与背景的对比度；
2. `p05` = 逐像素对比度的 5 分位，量「纸有多少比例已经读不出来」；
3. 判据用 **1**（纸/背景），3:1 是 WCAG 对非文本图形的常用门槛。

用法：
    python ui/scripts/iconsep.py            # 量 src-tauri/icons 里的 32x32 再缩到 16
    python ui/scripts/iconsep.py <png>
"""

import sys
from pathlib import Path

from PIL import Image, ImageDraw

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
DEFAULT = ROOT / "ui" / "src-tauri" / "icons" / "32x32.png"

# Windows 任务栏两套主题的底色
BACKGROUNDS = {
    "light #F3F3F3": (0xF3, 0xF3, 0xF3),
    "dark  #1F1F1F": (0x1F, 0x1F, 0x1F),
}

TRAY_PX = 16      # 托盘/任务栏的真实尺寸
TARGET = 3.0      # WCAG 非文本图形门槛


def lum(rgb):
    """WCAG 相对亮度。"""
    out = []
    for c in rgb[:3]:
        s = c / 255.0
        out.append(s / 12.92 if s <= 0.04045 else ((s + 0.055) / 1.055) ** 2.4)
    r, g, b = out
    return 0.2126 * r + 0.7152 * g + 0.0722 * b


def ratio(a, b):
    la, lb = lum(a), lum(b)
    hi, lo = max(la, lb), min(la, lb)
    return (hi + 0.05) / (lo + 0.05)


def measure(path, size):
    im = Image.open(path).convert("RGBA").resize((size, size), Image.LANCZOS)
    px = im.load()
    opaque = [
        px[x, y][:3]
        for y in range(size)
        for x in range(size)
        if px[x, y][3] >= 128  # 半透明边缘不参与：抗锯齿本身没有确定颜色
    ]
    # 量具自检：**双侧区间**，不是单侧阈值。
    #
    # 下界防「量的是空气」——内容整体塌掉、或压根没渲染出来。
    # 上界防的正是票 29 记的那次翻车：把缩在角落的图再缩小，
    # 截图尺寸照样是标称值，肉眼完全看不出来。
    #
    # ⚠ 只写下界等于没校准：实测填充是 25%，单侧阈值哪怕写 0.20，
    #   也只是「恰好没炸」。纸边长 286/512 ⇒ 面积占比 0.3125，
    #   勾探出纸外再收一点，16px 上阈值化到 25% 合理；
    #   于是区间取 0.18..0.42，两侧都留得下余量。
    fill = len(opaque) / (size * size)
    if not (0.18 <= fill <= 0.42):
        raise SystemExit(
            f"ABORT: opaque fill {fill:.0%} at {size}px is outside the plausible band "
            f"18%-42%. Too low = measuring air (render collapsed). Too high = content "
            f"over-fills the canvas, which is the icongen bbox trap from ticket 29."
        )

    paper = tuple(sorted(c[i] for c in opaque)[len(opaque) // 2] for i in range(3))
    print(f"  canvas={size}px  opaque={len(opaque)}/{size * size} ({fill:.0%})")
    print(f"  paper(median rgb)=rgb{paper}")

    results = {}
    for label, bg in BACKGROUNDS.items():
        rs = sorted(ratio(c, bg) for c in opaque)
        p05 = rs[max(0, int(len(rs) * 0.05) - 1)]
        med = rs[len(rs) // 2]
        results[label] = (med, p05)
        print(f"  {label}:  median={med:>6.2f}:1   p05={p05:>5.2f}:1   "
              f"(paper/bg={ratio(paper, bg):>6.2f}:1)")

    light = results["light #F3F3F3"][0]
    dark = results["dark  #1F1F1F"][0]
    ok = light >= TARGET
    print(f"\n  verdict: light median = {light:.2f}:1 (target >= {TARGET}:1) "
          f"-> {'OK' if ok else 'INSUFFICIENT'}")
    print(f"  guard: dark = {dark:.2f}:1 - the change must NOT degrade this")
    if not ok and dark < TARGET:
        print("  note: both sides fail -> this is not a 'light theme only' problem")
    return ok


def sweep(paper_rgb, bg_rgb, target=TARGET):
    """求「纸要压暗到多深」才够分离度 —— 只给数值，不改文件。

    为什么不做成直接调 `box-shadow`：票 29 原先建议的是一道**投影**，
    而投影在 16px 上是**亚像素**的（1024 开窗的 3px 投影缩到 16px 只剩
    0.06px），量出来等于没有。16px 上能立住的只有**纸本身的调子**。
    ⚠ 这是同一个坑的第二次：先写「看起来加了阴影会好些」，量下来是零。
    """
    print(f"  sweep: paper rgb{paper_rgb} vs bg rgb{bg_rgb}, target {target}:1")
    best = None
    for i in range(100, 39, -1):
        k = i / 100
        c = tuple(min(255, round(v * k)) for v in paper_rgb)
        r = ratio(c, bg_rgb)
        if r >= target:
            best = (k, c, r)
            break
        if i == 90:
            first = (k, c, r)
    if best is None:
        print(f"    never reaches {target}:1 even at 40% brightness "
              f"(at 90% it was {first[2]:.2f}:1)")
        return None
    k, c, r = best
    print(f"    brightness {k:.2f} -> rgb{c}  ratio={r:.2f}:1")
    print(f"    (10% darker would be {ratio(tuple(min(255, round(v * (k + .1))) for v in paper_rgb), bg_rgb):.2f}:1)")
    return best


def strip(paths, out, zoom=8):
    """把若干候选并成一张对照图，两种背景各一行。

    **量过 ≠ 好看。** 数字只回答「分离度够不够」，不回答「这张图标
    还像不像那张纸」——而后者正是不能由指标决定的那部分。16px 用
    `NEAREST` 放大，放大后看到的就是托盘上的真实像素排列。
    """
    tiles = []
    for p in paths:
        im = Image.open(p).convert("RGBA").resize((TRAY_PX, TRAY_PX), Image.LANCZOS)
        tiles.append((p.parent.name, im))

    cell = TRAY_PX * zoom
    pad = 10
    label_w = 150
    W = label_w + len(tiles) * (cell + pad) + pad
    H = pad + 2 * (cell + pad) + 30
    sheet = Image.new("RGB", (W, H), (255, 255, 255))
    d = ImageDraw.Draw(sheet)

    for row, (label, bg) in enumerate(BACKGROUNDS.items()):
        y = pad + row * (cell + pad)
        for i, (_, im) in enumerate(tiles):
            x = label_w + pad + i * (cell + pad)
            sheet.paste(bg, (x, y, x + cell, y + cell))
            big = im.resize((cell, cell), Image.NEAREST)
            sheet.paste(big, (x, y), big)
        d.text((6, y + cell // 2 - 4), label, fill=(0, 0, 0))

    for i, (name, _) in enumerate(tiles):
        d.text((label_w + pad + i * (cell + pad), H - 16), name[:18], fill=(0, 0, 0))
    d.text((6, H - 16), f"{TRAY_PX}px x{zoom}", fill=(0, 0, 0))
    sheet.save(out)
    print(f"  wrote {out}  ({W}x{H})")


if __name__ == "__main__":
    p = Path(sys.argv[1]) if len(sys.argv) > 1 else DEFAULT
    if len(sys.argv) == 1:
        print(f"{p.name}  (from {p.parent.name})")
        measure(p, TRAY_PX)
        print("\n-- sweep: how dark must the paper get --")
        sweep((246, 231, 191), BACKGROUNDS["light #F3F3F3"])
    elif p.name == "--strip":
        cand = sorted(Path(sys.argv[2]).glob("*/32x32.png"))
        if not cand:
            raise SystemExit(f"ABORT: no */32x32.png under {sys.argv[2]}")
        strip(cand, Path(sys.argv[3]))
    else:
        print(f"{p.name}  (from {p.parent.name})")
        measure(p, TRAY_PX)
