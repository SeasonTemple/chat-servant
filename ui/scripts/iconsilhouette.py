#!/usr/bin/env python3
"""量**整体剪影**在两种任务栏上的分离度 —— 双色分置方案的判据（票 34）。

为什么 iconsep 在这个方案下量不到东西
--------------------------------------
`iconsep.py` 取不透明像素的**中位色**问「纸自己能不能读出来」。
这套判据是为**单一载体**设计的：整张图标只有一种可读元素。

而票 34 重做后的方案是**双色分置** ——
    · 浅色任务栏靠**深墨描边**跳出来
    · 深色任务栏靠**奶白纸**跳出来
两侧各由**不同元素**负责。于是：
    · 浅底上纸的中位色必然只有 1.11:1（奶白 vs 近白）—— 但图标整体可读
    · iconsep 报 FAIL，量的是**纸**，而该读的是**描边**
⚠ 这就是「指标名与它量到的东西对不上」：不是方案不行，是尺子量错了部位。

本量具问的是那个真的问题：**整张图标有多大比例读得出来。**

判据（**刻意不对称**，理由见下）
--------------------------------
对每个 `alpha >= 128` 的像素，算它与底色的 WCAG 对比度，然后**分侧**报：

| 侧 | 门槛 | 问的是 |
|---|---|---|
| 深色 `#1F1F1F` | `median >= 8:1` | 能不能**跳出来** —— 这是本机 99% 用户实际看到的那一侧 |
| 浅色 `#F3F3F3` | `readable >= 30%` | 有没有可读的轮廓元素 —— 只需**不糊**，是保底 |

⚠⚠ **为什么不能两侧用同一个门槛** —— 这是本量具第一版的真实翻车。
第一版两侧都要求 `readable >= 50%`，结果**现状那张（`sat1`）拿了满分**：
两侧 median 3.91 / 3.80、readable 100% / 100%。
可它是**一整片中灰**，没有任何深墨轮廓 —— 正是票 34 用户抱怨的「一团灰」。

⇒ 对称门槛**奖励中间调**：中灰在任何一侧都能凑够 3:1，
   而 WCAG 3:1 是「非文本可辨识」的**底线**，不是「图标跳出来」的目标。
   ⇒ 「可读比例」单独看**测不出有没有形状**，必须一边报 `ink share`。

⚠ **深色侧优先是设计取舍，不是普适标准。** 依据是票 34 记的本机事实
   （`AppsUseLightTheme=0`，用户实际看到的是深色那一侧）。
   浅色侧只求不糊，是给少数浅色主题用户保底。
   **换一台默认浅色任务栏的机器，这两条门槛要反过来。**

用法：python ui/scripts/iconsilhouette.py <png> [<png> ...]
"""

import sys
from pathlib import Path

from PIL import Image

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

TRAY_PX = 16
TARGET = 3.0
BACKGROUNDS = {
    "light #F3F3F3": (0xF3, 0xF3, 0xF3),
    "dark  #1F1F1F": (0x1F, 0x1F, 0x1F),
}
INK_LUM = 0.08     # 比最暗那档任务栏还暗，才算「撑轮廓的墨」

# 刻意不对称，见模块 docstring
GATE = {
    "dark  #1F1F1F": ("median", 8.0),   # 跳出来
    "light #F3F3F3": ("readable", 0.30),  # 不糊即算过
}


def lum(rgb):
    out = []
    for c in rgb[:3]:
        s = c / 255.0
        out.append(s / 12.92 if s <= 0.04045 else ((s + 0.055) / 1.055) ** 2.4)
    r, g, b = out
    return 0.2126 * r + 0.7152 * g + 0.0722 * b


def ratio(a, b):
    hi, lo = max(a, b), min(a, b)
    return (hi + 0.05) / (lo + 0.05)


def measure(path):
    im = Image.open(path).convert("RGBA").resize((TRAY_PX, TRAY_PX), Image.LANCZOS)
    px = im.load()
    opaque = [
        px[x, y][:3]
        for y in range(TRAY_PX)
        for x in range(TRAY_PX)
        if px[x, y][3] >= 128
    ]
    if not opaque:
        raise SystemExit(f"ABORT: {path.name} has no opaque pixel")

    lums = [lum(c) for c in opaque]
    ink_share = sum(1 for v in lums if v <= INK_LUM) / len(lums)

    print(f"  {path.parent.name:<14} opaque={len(opaque):>3}/256  "
          f"ink={ink_share:5.1%}")
    ok = True
    for label, bg in BACKGROUNDS.items():
        b = lum(bg)
        rs = sorted(ratio(v, b) for v in lums)
        med = rs[len(rs) // 2]
        good = sum(1 for r in rs if r >= TARGET) / len(rs)
        kind, gate = GATE[label]
        got = med if kind == "median" else good
        ok_side = got >= gate
        ok = ok and ok_side
        print(f"    {label}:  median={med:>6.2f}:1   "
              f"readable(>={TARGET}:1)={good:6.1%}   "
              f"[{kind} {got:.2f} vs gate {gate}] {'OK' if ok_side else 'FAIL'}")
    return ok


def main():
    paths = [Path(p) for p in sys.argv[1:]]
    if not paths:
        raise SystemExit("usage: python ui/scripts/iconsilhouette.py <png> [<png> ...]")
    all_ok = True
    for p in paths:
        all_ok = measure(p) and all_ok
        print()
    print("PASS needs BOTH: dark median >= 8:1 (it must POP) "
          "AND light readable >= 30% (has a legible edge).")
    print("NOTE: '100% readable' alone proves NOTHING - a flat mid-grey slab")
    print("      scores 100% on both sides and still reads as a smudge.")
    return 0 if all_ok else 1


if __name__ == "__main__":
    sys.exit(main())