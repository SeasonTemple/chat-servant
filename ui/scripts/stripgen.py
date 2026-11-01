"""从面板自己的纸上裁下 1:1 全宽真撕边与连续纸芯，生成候选行的九宫格位图 —— 票 44 / 票 90。

## 2026-11-01 重构（消掉内嵌深色方框、四角 90° 直角块与 5× 横向拉伸，保留纸条可辨识度）

旧版 `strip.png` 只有 `128×128`（中段仅 64px 宽），在真机上造成三重「方框感」：
1. **5× 横向拉伸**：`.cs-act` 在 @2x 下宽 632px，64px 的中段被 `border-image ... stretch`
   横向拉了近 10 倍，原本参差的撕口被拉成水平平台段，纸芯里的暗斑被拉成横条；
2. **内芯与外圈分两支笔调色**：`recolour()` 对中央 `64×64` 保留 100% 暗斑（最暗 B=167），
   对四周 `32px` 边框带却压到 `DESHADE_KEEP = 0.08`（最暗 B=189），在 `x/y = 32, 96`
   处留下一圈色差 22 的矩形硬台阶，渲染后每条候选内部 16px 处都有一个清晰的内嵌深色方框；
3. **四角取单点 `CORNER_AT = 32`**：`build_corner` 沿边方向不随 `x/y` 变化，直接挤出四个
   `16×16` 的 90° 几何直角块。

现在的做法（`632×160`，`SLICE = 32` 源 px = `16 CSS px` 保持不变）：
- **RGB 单一连续净区 + 微提亮 +3**：整张 `632×160` 的 RGB 直接取自 `probe-sheet.png` 中部同一块连续净区
  `(64..696, 260..420)`，均值对齐到 `STRIP_RGB = (246, 233, 196)`（比底纸 `243,230,192` 亮 3 个单位，
  让纸条贴在底纸上时自身与底纸有层次）并保留真实纸纹纤维起伏（0.85×）。
  因为外圈 32px 与内芯 `fill` 区本来就是同一张连续纸面，**在 `SLICE = 32` 处天然零色差、零接缝**；
- **Alpha 1:1 全宽真撕口 + 最参差左右窗 + 连续相交角**：上下边直接取源图 632px 全宽真撕口的 alpha 通道，
  左右边取源图参差度最大的两段（左 `y=24..184`、右 `y=855..1015`，跨度均为 23 源 px，避免中段平直段像竖线），
  四角由横竖两条真撕口逐点 `min(a_h(x, y), a_v(x, y))` 连续相交，与中段零落差衔接。
"""
from __future__ import annotations

import sys
from pathlib import Path

import numpy as np
from PIL import Image

for _stream in (sys.stdout, sys.stderr):
    if hasattr(_stream, "reconfigure"):
        _stream.reconfigure(encoding="utf-8", errors="replace")

HERE = Path(__file__).resolve().parent.parent
SRC = HERE / "static" / "probe-sheet.png"
OUT = HERE / "static" / "strip.png"
REPORT = HERE.parent / "target" / "probe" / "stripgen-report.txt"

SLICE = 32
OUT_W = 632
OUT_H = 160
STRIP_RGB = np.array([246.0, 233.0, 196.0], dtype=np.float32)
GRAIN_KEEP = 0.85


def build() -> None:
    if not SRC.exists():
        sys.exit(f"找不到 {SRC}")

    src = np.asarray(Image.open(SRC).convert("RGBA"))
    sh, sw = src.shape[:2]
    if (sw, sh) != (760, 1120):
        sys.exit(f"源纸尺寸异常: {(sw, sh)} != (760, 1120)")

    # 1. RGB：取自面板中段连续净区，内外圈同源零接缝，比底纸微亮 +3 并保留 0.85× 真实纸纹
    x0, y0 = 64, 260
    rgb_crop = src[y0 : y0 + OUT_H, x0 : x0 + OUT_W, :3].astype(np.float32)
    mean_rgb = rgb_crop.mean(axis=(0, 1))
    rgb_out = np.clip(
        np.round(STRIP_RGB + (rgb_crop - mean_rgb) * GRAIN_KEEP), 0, 255
    ).astype(np.uint8)

    # 2. Alpha：上下取 632px 全宽真撕口，左右取源图参差度最大的两段（跨度 23 源 px），四角逐点连续求交
    a_top = src[0:SLICE, x0 : x0 + OUT_W, 3].copy()
    a_bot = src[sh - SLICE - 8 : sh - 8, x0 : x0 + OUT_W, 3].copy()
    a_left = src[24 : 24 + OUT_H, 0:SLICE, 3].copy()
    a_right = src[855 : 855 + OUT_H, sw - SLICE : sw, 3].copy()

    alpha = np.full((OUT_H, OUT_W), 255, dtype=np.uint8)
    a_top[-1, :] = np.maximum(a_top[-1, :], 254)
    a_bot[0, :] = np.maximum(a_bot[0, :], 254)
    alpha[0:SLICE, :] = np.minimum(alpha[0:SLICE, :], a_top)
    alpha[OUT_H - SLICE : OUT_H, :] = np.minimum(alpha[OUT_H - SLICE : OUT_H, :], a_bot)
    alpha[:, 0:SLICE] = np.minimum(alpha[:, 0:SLICE], a_left)
    alpha[:, OUT_W - SLICE : OUT_W] = np.minimum(alpha[:, OUT_W - SLICE : OUT_W], a_right)

    # 3. 自检：中段内侧必须实心，且 SLICE=32 接缝处内外色差 <= 2.5
    assert int(alpha[SLICE - 1, SLICE : OUT_W - SLICE].min()) >= 240, "顶边 SLICE 内侧未闭合"
    assert int(alpha[OUT_H - SLICE, SLICE : OUT_W - SLICE].min()) >= 240, "底边 SLICE 内侧未闭合"
    seam_diff = float(
        np.abs(
            rgb_out[SLICE, SLICE : OUT_W - SLICE].astype(np.float32)
            - rgb_out[SLICE - 1, SLICE : OUT_W - SLICE].astype(np.float32)
        ).mean()
    )
    assert seam_diff < 2.5, f"SLICE 接缝处存在色差台阶: {seam_diff:.2f}"

    out = np.dstack([rgb_out, alpha])
    Image.fromarray(out, "RGBA").save(OUT)
    REPORT.parent.mkdir(parents=True, exist_ok=True)
    REPORT.write_text(
        f"strip.png {OUT_W}x{OUT_H} slice={SLICE} seam_diff={seam_diff:.3f}\n",
        encoding="utf-8",
    )
    print(f"written: {OUT} ({OUT_W}x{OUT_H}, slice={SLICE}, seam_diff={seam_diff:.3f})")


if __name__ == "__main__":
    build()
