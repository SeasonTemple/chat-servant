#!/usr/bin/env python3
"""核验 `feMorphology` 描边到底有没有真的渲染出来（票 34 候选 A 的自检）。

为什么需要单独核
----------------
候选 A 出图后肉眼看着「描边几乎看不见」。有两种可能，处置完全相反：
  ① 描边没渲染 —— 实现坏了；
  ② 描边渲染了但 16px 上不够粗 —— 参数问题。
而这两者在成图上**看起来一模一样**，所以不能靠眼睛分。

⇒ 量两件事：
   1. **覆盖面积**：描边是向外膨胀，描边生效时 alpha≥128 的像素必然变多。
      这是「有没有生效」的直接证据，与粗细无关。
   2. **深墨像素数**：`PAPER_STROKE_COLOR` 是 #3A2F1E，量暗像素占比，
      这是「渲出来有多粗」的证据。

⚠ 别拿图标标的 bbox 面积当覆盖率——那量的是主墨迹，不是描边。

用法：python ui/scripts/strokeprobe.py
"""

import sys
from pathlib import Path

from PIL import Image

ROOT = Path(__file__).resolve().parents[2]
VAR = ROOT / "target" / "iconvar"
TRAY_PX = 16
INK_MAX = 90          # #3A2F1E 的 G=47，远低于 90；纸的 G=231 远高于


def stats(label, src_dir):
    im = Image.open(src_dir / "icon.png").convert("RGBA").resize(
        (TRAY_PX, TRAY_PX), Image.LANCZOS)
    px = im.load()
    opaque = ink = 0
    for y in range(TRAY_PX):
        for x in range(TRAY_PX):
            r, g, b, a = px[x, y]
            if a >= 128:
                opaque += 1
                if g <= INK_MAX:
                    ink += 1
    print(f"  {label:<22} opaque={opaque:>3}/256  ink={ink:>3}  ink/opaque={ink / max(opaque, 1):5.1%}")
    return opaque, ink


def main():
    print(f"tray canvas {TRAY_PX}x{TRAY_PX}, opaque counted at alpha>=128")
    base_o, base_i = stats("REGRESS (stroke=0)", VAR / "REGRESS")
    a_o, a_i = stats("A-stroke32", VAR / "A-stroke32")
    c_o, c_i = stats("C-both", VAR / "C-both")

    print()
    if a_o > base_o or a_i > base_i:
        print(f"=> stroke IS rendering: opaque {base_o} -> {a_o} "
              f"(+{a_o - base_o}), ink {base_i} -> {a_i} (+{a_i - base_i})")
    else:
        print("=> stroke is NOT rendering: coverage identical to stroke=0.")
        print("   The feMorphology filter is not taking effect - implementation bug,")
        print("   NOT a 'too thin' problem. Do not tune the radius, fix the filter.")

    print()
    if base_i == 0 and a_i <= 3:
        print(f"   ink share = {a_i / max(a_o, 1):.1%}: the edge exists but is too thin")
        print("   to survive the 64:1 shrink down to the 16px tray. Raise --stroke.")
    return 0


if __name__ == "__main__":
    sys.exit(main())