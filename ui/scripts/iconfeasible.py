#!/usr/bin/env python3
"""算「两侧任务栏都过 3:1」时纸亮度到底被锁在哪个区间里（票 34）。

为什么先算再扫
----------------
票 34 的三条候选里，两条（保持 0.52 / 补 PAPER_SATURATE）都在同一个前提
下成立：**纸的对比度只由亮度决定**。饱和度改色相、不改相对亮度，所以
在 WCAG 对比度这个判据上它是**免费的**——但也正因为免费，它救不了
「一团灰」之外的任何东西。

本脚本把那个前提算清楚：给定两种任务栏底色，求纸相对亮度的可行区间。
区间窄 ⇒ 亮度这条路已经走到头，剩下的杠杆只有**色相**与**载体**。

⚠ 与本项目另外两个量具同一批坑：
1. 用 WCAG 相对亮度，不是 0-255 平均——后者会把近白/近黑两端骗得很惨；
2. 区间端点要**实算**不要手推：下面每个数都是 `lum()` 跑出来的。

用法：python ui/scripts/iconfeasible.py
"""

import sys
from pathlib import Path

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

BACKGROUNDS = {
    "light #F3F3F3": (0xF3, 0xF3, 0xF3),
    "dark  #1F1F1F": (0x1F, 0x1F, 0x1F),
}
TARGET = 3.0


def lum(rgb):
    out = []
    for c in rgb[:3]:
        s = c / 255.0
        out.append(s / 12.92 if s <= 0.04045 else ((s + 0.055) / 1.055) ** 2.4)
    r, g, b = out
    return 0.2126 * r + 0.7152 * g + 0.0722 * b


def ratio(l1, l2):
    hi, lo = max(l1, l2), min(l1, l2)
    return (hi + 0.05) / (lo + 0.05)


def main():
    ls = {k: lum(v) for k, v in BACKGROUNDS.items()}
    print("Taskbar relative luminance (computed, not hand-derived):")
    for k, v in ls.items():
        print(f"  {k}: L = {v:.4f}")

    # 纸比浅底暗 ⇒ (Llight+0.05)/(Lpaper+0.05) >= T
    # 纸比深底亮 ⇒ (Lpaper+0.05)/(Ldark+0.05)  >= T
    hi_bound = (ls["light #F3F3F3"] + 0.05) / TARGET - 0.05   # 纸亮度的上界
    lo_bound = TARGET * (ls["dark  #1F1F1F"] + 0.05) - 0.05    # 纸亮度的下界
    print(f"\nFeasible band: Lpaper in [{lo_bound:.4f}, {hi_bound:.4f}] "
          f"(width {hi_bound - lo_bound:.4f})")

    if lo_bound > hi_bound:
        print("\n>>> EMPTY band: the two taskbars cannot both pass 3:1 at any")
        print("    paper luminance. Changing the carrier is the only route left.")
        return 1

    cur = lum((132, 121, 100))
    print(f"\nCurrent paper at brightness 0.52, rgb(132,121,100): L = {cur:.4f}")
    print(f"  margin above lower bound {cur - lo_bound:+.4f}, "
          f"below upper bound {hi_bound - cur:+.4f}")
    mid = (lo_bound + hi_bound) / 2
    print(f"  band midpoint L = {mid:.4f}; current sits {cur - mid:+.4f} off it")

    print("\nSweep the band:")
    print("  Lpaper    light/bg   dark/bg    verdict")
    for i in range(11):
        lp = lo_bound + (hi_bound - lo_bound) * i / 10
        rl = ratio(ls["light #F3F3F3"], lp)
        rd = ratio(lp, ls["dark  #1F1F1F"])
        print(f"  {lp:.4f}   {rl:6.2f}:1  {rd:6.2f}:1   "
              f"{'both OK' if min(rl, rd) >= TARGET else 'FAIL'}")

    print(f"\nMax balanced ratio over the whole interval: "
          f"{min(ratio(ls['light #F3F3F3'], mid), ratio(mid, ls['dark  #1F1F1F'])):.2f}:1 "
          f"(interval width {hi_bound - lo_bound:.4f}).")
    print("=> `brightness` has exactly ONE solution set; both taskbars pinch it")
    print("   from both sides. Tuning it further can only trade one side for the")
    print("   other. The remaining levers are HUE (saturate) and CARRIER.")
    return 0


if __name__ == "__main__":
    sys.exit(main())