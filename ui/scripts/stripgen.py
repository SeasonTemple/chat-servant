"""生成「裁剪纸条」——票 44 候选行动行的新零件。

## 为什么要有这个零件

候选行动行一直是**按钮**：1px 描边 + radius 4（`PaperPanel.svelte` 的 `.cs-act`）。
但这个面板的其余部分是一张**撕边纸**，上面有手绘笔迹。项目规范 §4.2b v2
早就否决过「按钮拿现成色块当零件」，而 §4.2c 定的按钮族恰恰是 Web 按钮
语言——**规范两节自己打架**，票 44 的根因就在这个交叉点上。

用户提的方向是「**裁剪**的纸布作为每个候选的横条」。注意「裁剪」：
- 裁切边是**直的**，所以不需要撕口那五层侧壁关系（§2.1），
  也就绕开了 §9 否掉的「用 CSS 分层去凑纸边」；
- 但它仍然有**厚度**——纸被裁开的那一刀会露出纸芯的密度，
  以及投在下面那张纸上的一线影。这两样让它读成「一片纸」而不是「一个矩形」。

## 为什么用 9-slice 而不是给每种行数各烘一张

候选一行 / 两行 / 三行都要用同一块纸。九宫格（CSS `border-image`）让
**中间的纸芯横向拉伸、四条边保持不动**，于是任意宽度都还是同一块纸。

⚠ 拉伸会把纸纹的横向特征拉长。纸纹是细颗粒噪声，横向拉伸在 380px 的
内容栏里肉眼不可见——这是选**横向**拉伸的原因（纵向拉伸会把颗粒拉成
明显的条纹）。若哪天出现可见的拉伸纹理，先换 `stretch → round`，
而不是回退去烘 N 张图。

## 输入输出

输入：`static/probe-sheet.png`（760×1120，RGBA，撕边已烘进位图）
输出：`static/strip.png`（RGBA，九宫格源图，@2x）

⚠ 纸的**原始生成方式已不可考**（设计规范 §2.4 🔴），但位图可复现——
所以本脚本是**从现有位图上裁**，不重新合成纸纹。
"""

from __future__ import annotations

import sys
from pathlib import Path

from PIL import Image, ImageDraw, ImageEnhance

HERE = Path(__file__).resolve().parent.parent  # ui/
SRC = HERE / "static" / "probe-sheet.png"
OUT = HERE / "static" / "strip.png"

# 九宫格每边多少像素（源图 @2x）。40 源像素 = 20 CSS px。
SLICE = 40
SRC_W = SRC_H = SLICE * 3  # 120×120 的源图，够切

# 裁切边上那几刀。数值是量出来的观感，不是设计规范里的数：
# 裁口内侧 2px（@2x）略深 = 纸芯被切断的密度；再往里 1px 略亮 =
# 刀刃蹭亮的那一线。两个加起来在 380px 栏里只有 ~1.5 CSS px。
DARK = 1
LIGHT = 1


def build() -> None:
    if not SRC.exists():
        sys.exit(f"找不到 {SRC}")

    paper = Image.open(SRC).convert("RGBA")

    # ── 纸芯 ──────────────────────────────────────────────────────
    # 取纸面中央一块**没有撕口、没有边**的干净区域。
    # 用整张纸的中间 120×120：760/2-60=320, 1120/2-60=500
    core = paper.crop((320, 500, 320 + SRC_W, 500 + SRC_H))

    # 纸条比下面那张纸**略新一点**——同一张纸裁下来也会因为没有落过灰而
    # 亮一点点。⚠ 只提亮 4%：再亮就变成「高亮的卡片」，
    # 那正是 §4.2b 否掉的色块语言。
    core = ImageEnhance.Brightness(core).enhance(1.04)

    # ── 裁切边 ────────────────────────────────────────────────────
    img = core.copy()
    d = ImageDraw.Draw(img)

    # 四条边内侧各压一道「纸芯密度」，再往里一道「刀刃蹭亮」。
    # ⚠ 画在**内侧**而不是外侧：画外侧会改变纸条的实际尺寸，
    # 而尺寸是排版要量的量（§4.3 那套字号下限就依赖它）。
    for i in range(DARK):
        a = int(150 * (1 - i / max(DARK, 1)))
        d.rectangle([i, i, SRC_W - 1 - i, SRC_H - 1 - i], outline=(92, 76, 48, a), width=1)
    for j in range(DARK, DARK + LIGHT):
        a = int(110 * (1 - (j - DARK) / max(LIGHT, 1)))
        d.rectangle([j, j, SRC_W - 1 - j, SRC_H - 1 - j], outline=(255, 252, 240, a), width=1)

    img.save(OUT)

    print(f"源纸      : {SRC.name} {paper.size}")
    print(f"纸条      : {OUT.name} {img.size}  （九宫格 slice={SLICE}px @2x）")
    print(f"裁口      : 密度线 {DARK}px + 刀刃亮线 {LIGHT}px（@2x，≈1.5 CSS px）")
    print(f"纸芯      : 取自 ({320},{500}) 起的 {SRC_W}×{SRC_H} 干净区，亮度 ×1.04")


if __name__ == "__main__":
    build()
