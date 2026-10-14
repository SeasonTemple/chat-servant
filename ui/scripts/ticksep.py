#!/usr/bin/env python3
"""量红勾在两种任务栏背景上的**可辨识度**（票 29 遗留的「看见了但没量」）。

为什么单独一个量具
------------------
`iconsep.py` 量的是**纸**——不透明像素的**中位色**（中位色就是纸本身），
所以给纸加暗边/投影都动不了它。票 29 定案时记下一条欠账：
**16px 上红勾偏粉，可见但未量**。那是另一件事：红勾是 `TICK_NESTED_BIG`
里 5 条**半透明** path（opacity 0.22…0.58）叠在纸上，合成色既不是纯红
也不是纸，判据与纸完全不同。

「偏粉」是可证伪的量
--------------------
16px 上红勾像素的**色相与饱和度**若被纸（现已是压暗的深牛皮纸）拉灰，
它与**浅色任务栏**底的对比度会明显低于纸自己。要量的就是：
- `tick` = 红勾像素（在 16px 图上按「红相 + 非纸色」挑出）的对比度中位数；
- 与 `paper` 对比，**tick 在浅色侧必须仍然可分**。

⚠ 与 iconsep 同一批坑：
1. **半透明边缘（alpha<128）不参与**——抗锯齿像素没有确定颜色；
2. **必须用对比度，不是亮度差**——近白/近黑两端绝对亮度差会骗人；
3. **双侧区间自检**——像素数太少说明挑不出勾（量的是空气），
   太多说明把纸也挑进来了（量的不是勾）。

用法：python ui/scripts/ticksep.py [png]
"""

import sys
from pathlib import Path

from PIL import Image

ROOT = Path(__file__).resolve().parents[2]
DEFAULT = ROOT / "ui" / "src-tauri" / "icons" / "32x32.png"

BACKGROUNDS = {
    "light #F3F3F3": (0xF3, 0xF3, 0xF3),
    "dark  #1F1F1F": (0x1F, 0x1F, 0x1F),
}
TRAY_PX = 16
TARGET = 3.0

# 红勾相对纸的特征：R 明显高于 G/B，且比纸更「红」。
# 纸（压暗后 ~rgb(132,121,100)）R-B≈32；纯红 #c0392b R-B≈149。
# 取 60 作为「明显偏红」的门槛——把纸排除、保留被稀释的勾。
REDNESS_MIN = 60
REDNESS_MAX = 260


def lum(rgb):
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


def redness(rgb):
    """R 减 B。勾偏红 ⇒ 正值大；纸接近中性 ⇒ 小。"""
    return rgb[0] - rgb[2]


def measure(path, size):
    im = Image.open(path).convert("RGBA").resize((size, size), Image.LANCZOS)
    px = im.load()
    opaque = [
        px[x, y][:3]
        for y in range(size)
        for x in range(size)
        if px[x, y][3] >= 128
    ]
    if not opaque:
        raise SystemExit("ABORT: no opaque pixel - icon did not render")

    ticks = [c for c in opaque if REDNESS_MIN <= redness(c) <= REDNESS_MAX]
    paper = [c for c in opaque if redness(c) < REDNESS_MIN]

    # ── self-check: 双侧区间（tick 像素占比）
    #
    # 下界：太少 = 量的是空气（勾在 16px 上可能被压没了）。
    # 上界：太多 = 把纸/抗锯齿边也挑进来了，那量的不是「勾」。
    frac = len(ticks) / len(opaque)
    if not (0.03 <= frac <= 0.35):
        raise SystemExit(
            "ABORT: tick share of opaque pixels %.0f%% outside 3%%-35%%. "
            "Too low = the tick vanished at %dpx. Too high = paper or AA edge "
            "leaked into the 'tick' bucket, so this is not measuring the tick."
            % (100 * frac, size)
        )

    tick_med = tuple(sorted(c[i] for c in ticks)[len(ticks) // 2] for i in range(3))
    paper_med = tuple(sorted(c[i] for c in paper)[len(paper) // 2] for i in range(3))

    print(f"  canvas={size}px  opaque={len(opaque)}  tick={len(ticks)} ({frac:.0%})  paper={len(paper)}")
    print(f"  tick(median rgb)=rgb{tick_med}   paper(median rgb)=rgb{paper_med}")

    results = {}
    for label, bg in BACKGROUNDS.items():
        tick_rs = sorted(ratio(c, bg) for c in ticks)
        t_med = tick_rs[len(tick_rs) // 2]
        p_med = ratio(paper_med, bg)
        results[label] = (t_med, p_med)
        print(f"  {label}:  tick median={t_med:>6.2f}:1   paper/bg={p_med:>6.2f}:1")

    light_tick, light_paper = results["light #F3F3F3"]
    dark_tick, _ = results["dark  #1F1F1F"]
    ok = light_tick >= TARGET
    print(f"\n  verdict: LIGHT tick median = {light_tick:.2f}:1 (target >= {TARGET}:1) "
          f"-> {'OK' if ok else 'INSUFFICIENT'}")
    print(f"  guard: dark tick = {dark_tick:.2f}:1 (must not degrade).")
    print(f"  note: light paper/bg = {light_paper:.2f}:1 - the tick must stay at "
          f"least as separable as the paper it sits on, else it reads as a smudge.")
    if light_tick < light_paper:
        print("  >>> the tick is LESS separable than the paper on light: it is the smudge.")
    return ok


if __name__ == "__main__":
    p = Path(sys.argv[1]) if len(sys.argv) > 1 else DEFAULT
    print(f"{p.name}  (from {p.parent.name})")
    measure(p, TRAY_PX)