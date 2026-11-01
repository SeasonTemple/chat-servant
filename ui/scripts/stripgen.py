"""从面板自己的纸上裁下真撕边与纸纹折皱，生成候选行的 128×128 九宫格位图 —— 票 44 / 票 90。

## 2026-11-01 订正（保留原宽与纸面折皱，消掉内嵌 64×64 方框接缝、四角直角块与外沿残影）

1. **保持 128×128 与原有四边内缩深度（15~23 源 px）**：纸条宽度与原有折皱纹理（横向纤维起伏）
   完全保留，不会把纸条撑宽或抹平成塑料平涂；
2. **内外圈共用同一张 128×128 中心纸样的 RGB**：整张 128×128 的 RGB 都取自 `probe-sheet.png`
   中心 `128×128` 纸样（均值对齐到 `PANEL_RGB = (243, 230, 192)`，保留原有折皱起伏，仅夹取极端
   污斑 `±14`），因此 `x/y = 32, 96`（`SLICE = 32`）处内外圈天然零台阶，消掉内嵌深色方框；
3. **四角连续求交 + 清掉外圈 8 源 px 残影**：四角由横竖两条真撕口逐点 `min(a_h(x, y), a_v(x, y))`
   连续相交（不再取单点 `CORNER_AT` 挤出 90° 直角块），并将 `alpha < 24` 与外沿 8 源 px 清零，
   确保撕口外侧完全透明。
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

SIDE = 128
SLICE = 32
PANEL_RGB = np.array([243.0, 230.0, 192.0], dtype=np.float32)


def build() -> None:
    if not SRC.exists():
        sys.exit(f"找不到 {SRC}")

    paper = Image.open(SRC).convert("RGBA")
    sw, sh = paper.size
    if (sw, sh) != (760, 1120):
        sys.exit(f"源纸尺寸异常: {(sw, sh)} != (760, 1120)")

    # 1. RGB：整张 128×128 取自中心纸样，保留原有折皱纹理且 SLICE=32 接缝处零台阶
    cx0, cy0 = (sw - SIDE) // 2, (sh - SIDE) // 2
    core_patch = np.asarray(
        paper.crop((cx0, cy0, cx0 + SIDE, cy0 + SIDE)), dtype=np.float32
    )[:, :, :3]
    core_mean = core_patch[SLICE : SIDE - SLICE, SLICE : SIDE - SLICE].mean(axis=(0, 1))
    rgb_full = np.clip(
        PANEL_RGB + (core_patch - core_mean), PANEL_RGB - 14.0, PANEL_RGB + 14.0
    )
    rgb_out = np.clip(np.round(rgb_full), 0, 255).astype(np.uint8)

    # 2. Alpha：沿用原版四条最参差撕口窗口（保持原有纸条宽度与内缩），四角逐点连续求交
    src_a = np.asarray(paper)[:, :, 3]
    a_top = src_a[0:SLICE, 606 : 606 + SIDE].copy()
    a_bot = src_a[sh - SLICE : sh, 185 : 185 + SIDE].copy()
    a_left = src_a[24 : 24 + SIDE, 0:SLICE].copy()
    a_right = src_a[855 : 855 + SIDE, sw - SLICE : sw].copy()

    alpha = np.full((SIDE, SIDE), 255, dtype=np.uint8)
    alpha[0:SLICE, :] = np.minimum(alpha[0:SLICE, :], a_top)
    alpha[SIDE - SLICE : SIDE, :] = np.minimum(alpha[SIDE - SLICE : SIDE, :], a_bot)
    alpha[:, 0:SLICE] = np.minimum(alpha[:, 0:SLICE], a_left)
    alpha[:, SIDE - SLICE : SIDE] = np.minimum(alpha[:, SIDE - SLICE : SIDE], a_right)
    alpha[alpha < 24] = 0
    alpha[0:8, :] = 0
    alpha[SIDE - 8 : SIDE, :] = 0
    alpha[:, 0:8] = 0
    alpha[:, SIDE - 8 : SIDE] = 0

    seam_diff = float(
        np.abs(
            rgb_out[SLICE, SLICE : SIDE - SLICE].astype(np.float32)
            - rgb_out[SLICE - 1, SLICE : SIDE - SLICE].astype(np.float32)
        ).mean()
    )
    assert seam_diff < 3.0, f"SLICE 接缝处存在色差台阶: {seam_diff:.2f}"

    out = np.dstack([rgb_out, alpha])
    Image.fromarray(out, "RGBA").save(OUT)
    REPORT.parent.mkdir(parents=True, exist_ok=True)
    REPORT.write_text(
        f"strip.png {SIDE}x{SIDE} slice={SLICE} seam_diff={seam_diff:.3f}\n",
        encoding="utf-8",
    )
    print(f"written: {OUT} ({SIDE}x{SIDE}, slice={SLICE}, seam_diff={seam_diff:.3f})")


if __name__ == "__main__":
    build()
