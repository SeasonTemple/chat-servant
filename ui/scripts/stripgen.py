"""从面板自己的纸上裁下真撕边、3D 边缘折皱与内芯纤维，生成候选行的位图 —— 票 44 / 票 90。

## 2026-11-01 五轮收敛（锐化上沿撕口 + 强化四边 3D 折皱 + 零内嵌方框）

1. **为什么之前纸条上边不够清晰、边框折皱不够**：
   - 源图顶边的 alpha 从 0 爬到 245 跨了 16 源 px（8 CSS px 的宽缓半透明带），而 `drop-shadow`
     是向下偏移的，导致纸条上边与同色底纸之间变成一团 8px 宽的雾，看不清上沿边界；
   - 之前把边缘折皱系数压在 `0.55~0.72` 且从 `d=16` 就开始向内羽化，把 `d=16..24` 处最明显的
     3D 边框折皱抹淡了一半。
2. **修法（`600×160`，`SLICE = 32`）**：
   - **锐化撕口 alpha 剖面（`sharpen_alpha`）+ 撕口纸厚边缘浮雕（`edge_bevel`）**：将四边半透明过渡带
     以 145 为中心收紧 2.4 倍，并在撕口边缘（`a ≈ 195`）叠加纸张厚度的微立体明暗，使纸条尤其是
     **上边（top edge）**轮廓清晰锐利；
   - **强化四边 3D 边框折皱（`keep = 1.20`，保持到 `d = 23`）**：完整保留 `d = 0..23` 的 3D 边框折皱，
     仅在最后 8 源 px（`d = 23..31`）用余弦羽化过渡到内芯纤维（`SLICE = 32` 接缝色差 `1.915 < 2.5`，
     无内嵌方框）。
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

OUT_W = 600
OUT_H = 160
SLICE = 32
PANEL_RGB = np.array([243.0, 230.0, 192.0], dtype=np.float32)


def norm_edge_rgb(edge_rgba: np.ndarray, keep: float, edge_bevel: float) -> np.ndarray:
    rgb = edge_rgba[:, :, :3]
    a = edge_rgba[:, :, 3]
    m = a > 210
    med = np.median(rgb[m], axis=0) if m.any() else PANEL_RGB
    alpha_gate = np.clip((a - 80.0) / 50.0, 0.0, 1.0)[:, :, None]
    dev = np.clip((rgb - med) * keep * alpha_gate, -46.0, 20.0)
    rim = np.exp(-0.5 * ((a - 195.0) / 32.0) ** 2)[:, :, None] * edge_bevel
    return np.clip(PANEL_RGB + dev + rim, 0, 255)


def feather_weight(d_idx: int) -> float:
    t = float(np.clip((d_idx - 23.0) / 8.0, 0.0, 1.0))
    return float(0.5 * (1.0 + np.cos(np.pi * t)))


def sharpen_alpha(a: np.ndarray) -> np.ndarray:
    return np.clip((a - 145.0) * 2.4 + 145.0, 0.0, 255.0)


def build() -> None:
    if not SRC.exists():
        sys.exit(f"找不到 {SRC}")

    src = np.asarray(Image.open(SRC).convert("RGBA"), dtype=np.float32)
    sh, sw = src.shape[:2]
    if (sw, sh) != (760, 1120):
        sys.exit(f"源纸尺寸异常: {(sw, sh)} != (760, 1120)")

    base_crop = src[260 : 260 + OUT_H, 80 : 80 + OUT_W, :3]
    base_mean = base_crop.mean(axis=(0, 1))
    crease_src = Image.fromarray(
        np.clip(src[480 : 480 + OUT_H, 320:440, :3], 0, 255).astype(np.uint8)
    ).resize((OUT_W, OUT_H), Image.BILINEAR)
    crease_arr = np.asarray(crease_src, dtype=np.float32)
    crease_dev = crease_arr - crease_arr.mean(axis=(0, 1))
    rgb_interior = np.clip(
        PANEL_RGB + (base_crop - base_mean) * 0.55 + crease_dev * 0.75,
        PANEL_RGB - 15.0,
        PANEL_RGB + 15.0,
    )

    top_rgba = src[0:SLICE, 80 : 80 + OUT_W].copy()
    top_rgba[:, 505:565, :] = top_rgba[:, 205:265, :]
    bot_rgba = src[sh - SLICE : sh, 80 : 80 + OUT_W].copy()
    left_rgba = np.zeros((OUT_H, SLICE, 4), dtype=np.float32)
    left_rgba[:, 4:SLICE, :] = src[545 : 545 + OUT_H, 0 : SLICE - 4, :]
    right_rgba = np.zeros((OUT_H, SLICE, 4), dtype=np.float32)
    right_rgba[:, 0 : SLICE - 4, :] = src[385 : 385 + OUT_H, sw - (SLICE - 4) : sw, :]

    top_rgb = norm_edge_rgb(top_rgba, keep=1.20, edge_bevel=-16.0)
    bot_rgb = norm_edge_rgb(bot_rgba, keep=0.95, edge_bevel=-12.0)
    left_rgb = norm_edge_rgb(left_rgba, keep=1.20, edge_bevel=-14.0)
    right_rgb = norm_edge_rgb(right_rgba, keep=1.20, edge_bevel=-14.0)

    rgb_out = rgb_interior.copy()
    for d in range(SLICE):
        w = feather_weight(d)
        rgb_out[d, :, :] = w * top_rgb[d, :, :] + (1.0 - w) * rgb_out[d, :, :]
        rgb_out[OUT_H - 1 - d, :, :] = (
            w * bot_rgb[SLICE - 1 - d, :, :] + (1.0 - w) * rgb_out[OUT_H - 1 - d, :, :]
        )
    for d in range(SLICE):
        w = feather_weight(d)
        rgb_out[:, d, :] = w * left_rgb[:, d, :] + (1.0 - w) * rgb_out[:, d, :]
        rgb_out[:, OUT_W - 1 - d, :] = (
            w * right_rgb[:, SLICE - 1 - d, :] + (1.0 - w) * rgb_out[:, OUT_W - 1 - d, :]
        )

    a_t = sharpen_alpha(top_rgba[:, :, 3])
    a_t[-1, :] = np.maximum(a_t[-1, :], 254)
    a_b = sharpen_alpha(bot_rgba[:, :, 3])
    a_b[0, :] = np.maximum(a_b[0, :], 254)
    a_l = sharpen_alpha(left_rgba[:, :, 3])
    a_l[:, -1] = np.maximum(a_l[:, -1], 254)
    a_r = sharpen_alpha(right_rgba[:, :, 3])
    a_r[:, 0] = np.maximum(a_r[:, 0], 0)

    alpha = np.full((OUT_H, OUT_W), 255.0, dtype=np.float32)
    alpha[0:SLICE, :] = np.minimum(alpha[0:SLICE, :], a_t)
    alpha[OUT_H - SLICE : OUT_H, :] = np.minimum(alpha[OUT_H - SLICE : OUT_H, :], a_b)
    alpha[:, 0:SLICE] = np.minimum(alpha[:, 0:SLICE], a_l)
    alpha[:, OUT_W - SLICE : OUT_W] = np.minimum(alpha[:, OUT_W - SLICE : OUT_W], a_r)
    alpha[alpha < 24] = 0
    alpha[0:6, :] = 0
    alpha[OUT_H - 6 : OUT_H, :] = 0
    alpha[:, 0:6] = 0
    alpha[:, OUT_W - 6 : OUT_W] = 0

    out = np.dstack(
        [
            np.clip(np.round(rgb_out), 0, 255).astype(np.uint8),
            np.clip(np.round(alpha), 0, 255).astype(np.uint8),
        ]
    )
    seam_diff = float(
        np.abs(
            out[SLICE, SLICE : OUT_W - SLICE, :3].astype(float)
            - out[SLICE - 1, SLICE : OUT_W - SLICE, :3].astype(float)
        ).mean()
    )
    assert seam_diff < 2.5, f"SLICE 接缝处存在色差台阶: {seam_diff:.2f}"

    Image.fromarray(out, "RGBA").save(OUT)
    REPORT.parent.mkdir(parents=True, exist_ok=True)
    REPORT.write_text(
        f"strip.png {OUT_W}x{OUT_H} slice={SLICE} seam_diff={seam_diff:.3f}\n",
        encoding="utf-8",
    )
    print(f"written: {OUT} ({OUT_W}x{OUT_H}, slice={SLICE}, seam_diff={seam_diff:.3f})")


if __name__ == "__main__":
    build()
