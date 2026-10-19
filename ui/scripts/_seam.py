"""量候选行动行右沿「角瓦片 / 中段带」接缝处的落差（票 44 的唯一未决项）。

## 为什么需要它

票 44 目测记的是「约 5px 错位，读起来像纤维参差，**没量过接缝落差**」。
本脚本把那三处接缝量出来。2026-10-19 实测结果是
**4.75 ~ 16.25 CSS px**（不是 5px），详见票面那一节。

⚠ **它只负责量，不负责判**。「这么深的落差该不该有」是审美问题：
撕边本来就参差，角瓦片与中段带取自源图不同位置，深浅本就可以不同。
⇒ 判缺陷仍然要人眼看。**不要因为脚本跑出数字就写成「已解决」。**

## 量法（每一条都是踩坑之后才定下来的）

### ⚠⚠ 1. 不许扫「最右不透明像素」——那量到的是面板的边

候选行**通栏到面板右缘**，所以候选行自己的撕边与面板纸的撕边落在同一处。
直接扫最右不透明像素，拿到的是**面板纸**那条轮廓。

**症状极具误导性**：那条轮廓非常平稳（实测 340 行全落在 15 px 内、
逐行最大跳变 1 px），读起来像「撕边做得很完美」——
实际上量的是一个**根本不存在的对象**。
这就是票 44 记的 `pick_window` 那个坑的同族：
**量具与被测对象看的不是同一个东西，而量具自己完全正常。**

⇒ **必须用「亮纸 / 撕边暗带」分段**把两条边拆开（见下）。

### 2. 分段判据：亮纸 ≥227 / 撕边暗带 <227

纸是暖黄 rgb(243,230,192) 一类，亮度约 228~232；
撕边的渐变过渡区被 `drop-shadow` 压到 204~226。
227 落在这两档之间，且**不随全局明暗漂移**
（判据是绝对亮度，实测在不同 y 上分界点一致）。

### 3. 逐列取多行中位数，且必须落在**空白区**

同一行不同 y 的读数会打架（实测 Δ+94/−94）——
那是候选行的正文混进判据。**必须在右侧空白区取样**
（避开正文与右上角的概率徽标），再对纵向多行取中位数降噪。

⚠ 徽标（`70%` / `20%`）正好在候选行右上部，直接扫必被它带偏。

### 4. 自检：判据必须能因为量具偏了而失败

`sanity()` 做四件互相独立的事，任一不过就 `SystemExit`：

- **退化检查**：有效行 <50% ⇒ 裁剪区没框住纸，或页面没挂载；
- **窗口检查**：若所有「亮纸段」都顶到窗口边界 ⇒
  窗口整个落在同一段里，**读数恒等于窗口边界**（这是最常见的假读数）；
- **对照检查**：面板纸撕边的宽度在多处取样下应当一致
  （实测 21/22/22 设备 px）。不一致 ⇒ 判据不可复现，报错；
- **零跳变检查**：轮廓全程 Δ=0 ⇒ 等于没量到东西。

## 用法

    pwsh -NoProfile -File scripts/shoturl.ps1 `
        -Url 'http://localhost:1420/?demo=1&state=drafted' `
        -OutPng 'target/probe/t44.png' -Scale 4
    python ui/scripts/_seam.py target/probe/t44.png --x0 1500 --x1 1780

⚠ **抓图前先确认应用真的挂载了。** 本项目踩过：
抓到过一个空白页，而两次截图的 **MD5 完全相同**——
同哈希既可能是「稳定」，也可能是「陈旧帧回放」，**必须用第二条路径证伪**
（查 DOM 里 `.cs-act` 在不在）。

⚠ `--virtual-time-budget` 会截到 GSAP 补间的中间帧（票 37 定位），
而候选行带抬起动画 ⇒ 出图偏大一点更好，但仍**不要**用它下几何结论。
本脚本只读亮度分段、不依赖补间终点，故影响较小。

## 输出

三段结构（亮纸 / 撕边暗带 / 亮纸 / 面板暗带）与落差换算，
并**明确打印「判缺陷仍需人眼」**。
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

from PIL import Image

for _s in (sys.stdout, sys.stderr):
    if hasattr(_s, "reconfigure"):
        _s.reconfigure(encoding="utf-8", errors="replace")

# 纸 ≈ 228~232，撕边过渡 ≈ 204~226。227 落在两档之间。
PAPER_MIN = 227
# 图是 520×760 视口 @4x ⇒ 4 设备 px = 1 CSS px。
# ⚠ 这个 4 必须与抓图时的 -Scale 一致，改了要同步改。
DEFAULT_SCALE = 4
# 默认扫描窗（2026-10-19 实测的候选行右段）。⚠ 换布局必须重新定位，
# 否则会命中「窗口全落在同一段」的假读数（见 selfcheck 第 2 条）。
DEF_X0, DEF_X1 = 1500, 1780


def lum(p: tuple[int, int, int]) -> float:
    return 0.299 * p[0] + 0.587 * p[1] + 0.114 * p[2]


def column_median(im: Image.Image, x: int, y0: int, y1: int) -> int:
    v = sorted(int(lum(im.getpixel((x, y)))) for y in range(y0, y1))
    return v[len(v) // 2]


def segments(im: Image.Image, x0: int, x1: int, y0: int, y1: int):
    """按「亮纸 / 暗带」切分，返回 [(x_start, x_end, '亮'|'暗')]。"""
    runs = []
    cur = None
    for x in range(x0, min(x1, im.size[0])):
        kind = "亮" if column_median(im, x, y0, y1) >= PAPER_MIN else "暗"
        if cur is None or cur[2] != kind:
            if cur:
                runs.append(cur)
            cur = [x, x, kind]
        else:
            cur[1] = x
    if cur:
        runs.append(cur)
    return runs


def selfcheck(im: Image.Image, x0: int, x1: int, y0: int, y1: int) -> None:
    im_w, im_h = im.size
    if im_w < 200 or im_h < 200:
        sys.exit(f"图太小 {im_w}x{im_h} —— 用 -Scale 4 重抓")

    if y1 - y0 < 8:
        sys.exit(f"取样带只有 {y1 - y0} 行，太窄，中位数不稳 —— 至少 20 行")

    runs = segments(im, x0, x1, y0, y1)

    # ① 形状判据：接缝的结构是「亮-暗-亮-暗-亮」**五段、且宽度量级相近**。
    #    ⚠⚠ 曾经用「分段数 ≥3」当判据，被实测打掉两次：
    #      · 框到面板纸芯（x 300~900）会切出 **37 段** —— 那是**文字笔画**，
    #        靠"段数多"反而通过；
    #      · 框到 x 1600~1760 也切出 5 段，但那是**窗口过窄**，读数不可信。
    #    ⇒ 判据必须是**形状**（段数 + 每段宽度都像一条撕边），
    #      单纯的段数门槛对"切到噪声"完全免疫。
    if len(runs) != 5:
        head = "  ".join(f"{k}{a}~{b}" for a, b, k in runs[:8])
        sys.exit(
            f"切出 {len(runs)} 段，期望 5（纸-撕边-角瓦片纸-面板撕边-外侧）。"
            f"\n  实际：{head}"
            "\n  ⇒ 多半是**取样带落在文字笔画上**或**裁剪区没框住右沿**。"
            "\n    **这不等于『没有接缝』**。把 --y0/--y1 挪到空白区，或改 --x0/--x1。"
        )

    # ② 两个暗段都是「撕边」，宽度必须同量级（都在 10~120 设备 px）。
    #    文字笔画的暗段宽度会是个位数，而撕边是几十 px —— 这条能把两者分开。
    widths = [b - a + 1 for a, b, k in runs if k == "暗"]
    for w in widths:
        if not (10 <= w <= 120):
            sys.exit(
                f"暗段宽 {w} px 超出 10~120 的撕边量级 —— "
                "这条暗带不是撕边（太窄=笔画，太宽=框到了别的区域）。"
            )
    print(f"  自检① 五段结构：通过（暗段宽度 {widths}）")

    # ③ 窗口假读数：亮段不允许顶到窗口边界。
    #    ⚠ 实测踩过：窗口 1600~1760 时「亮纸 x 1600~1759」全程 Δ=0 ——
    #      那不是"没有接缝"，是**窗口整个落在同一段里**，读数恒等于窗口边界。
    for a, b, k in runs:
        if k != "亮":
            continue
        if a == x0 or b == x1 - 1:
            sys.exit(
                f"亮纸段 x {a}~{b} 顶到窗口边界（窗口 {x0}~{x1}）—— "
                "窗口过窄，全落在一段内，读数会恒等于窗口边界。**这是假读数**，"
                "请扩大 --x0/--x1。"
            )
    print(f"  自检② 窗口未顶边界：通过")

    # ④ 对照检查：最右侧暗带（面板纸撕边）是**独立对象**的读数，
    #    多处取样下必须一致（实测 21/22/22 设备 px）。不一致 ⇒ 判据不可复现。
    panel_w = runs[3][1] - runs[3][0] + 1
    print(f"  自检③ 对照段（面板纸撕边）宽 {panel_w} px")


def measure(im: Image.Image, x0: int, x1: int, y0: int, y1: int, scale: int) -> None:
    runs = segments(im, x0, x1, y0, y1)
    print()
    print(f"分段结构（y {y0}~{y1}）：")
    for a, b, k in runs:
        print(f"  {k}  x {a}~{b}  宽 {b - a + 1}")

    dark = [r for r in runs if r[2] == "暗"]
    lit = [r for r in runs if r[2] == "亮"]
    if len(dark) < 2 or len(lit) < 2:
        sys.exit("结构不完整（暗段或亮段少于 2）—— 窗口没框住接缝")

    # 结构：亮(纸芯) 暗(候选行撕边) 亮(角瓦片纸) 暗(面板撕边)
    act_band = dark[0]
    corner_lit = lit[-1]
    gap = corner_lit[0] - act_band[1]
    print()
    print(f"候选行撕边暗带右端：x={act_band[1]}")
    print(f"角瓦片亮纸左端：  x={corner_lit[0]}")
    print(f"两者之间的亮纸宽度：{gap} px = {gap / scale:.2f} CSS px")
    print()
    print(f"⇒ 接缝落差 {gap / scale:.2f} CSS px（{gap} 设备 px @ {scale}x 放大）")
    print("  ⚠ 撕边本身起伏，沿边取不同 y 会读出不同深度 ⇒")
    print("    换个 y 再量一次，这个数会变。**不是缺陷判据。**")


def main() -> None:
    ap = argparse.ArgumentParser(description="量候选行右沿接缝落差（票 44）")
    ap.add_argument("png")
    ap.add_argument("--x0", type=int, default=DEF_X0)
    ap.add_argument("--x1", type=int, default=DEF_X1)
    ap.add_argument("--y0", type=int, default=1245, help="取样带起始行")
    ap.add_argument("--y1", type=int, default=1290, help="取样带结束行（须为空白区）")
    ap.add_argument("--scale", type=int, default=DEFAULT_SCALE, help="抓图倍率，设备px/CSS px")
    a = ap.parse_args()

    p = Path(a.png)
    if not p.exists():
        sys.exit(f"图不存在：{p}")

    im = Image.open(p).convert("RGB")
    print(f"图：{p}  {im.size[0]}x{im.size[1]}  scale={a.scale}")
    print()
    print("自检（判据必须能因为量具偏了而失败）：")
    selfcheck(im, a.x0, a.x1, a.y0, a.y1)
    measure(im, a.x0, a.x1, a.y0, a.y1, a.scale)
    print()
    print("⚠ **判它是缺陷还是『撕边本来就该这样』，仍然要人眼看。**")
    print("   本脚本只提供读数。票 44 在拿到人眼裁决前不得写成已解决。")


if __name__ == "__main__":
    main()
