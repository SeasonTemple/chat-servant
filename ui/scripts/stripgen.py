"""从面板自己的纸上裁下真撕边、3D 边缘折皱与内芯纤维，生成候选行的位图 —— 票 44 / 票 90。

## 2026-11-01 四轮收敛（保留原窄宽 + 边缘 3D 折皱 + 内芯纤维折皱，消掉内嵌方框与直角块）

1. **为什么之前「边缘折皱」没了**：
   - 旧 `stripgen.py` 用 `DESHADE_KEEP = 0.08` 把外圈 32px 的色差压掉了 92%，连同面板边缘
     原本画着的 V 形豁口明暗与 3D 边缘折皱（`top x=80..680`、`left y=545..705`、`right y=385..545`）
     一起抹成了平涂纯色；
   - 而 `128×128` 窄图被 `border-image ... stretch` 横向拉 5 倍后，原本斜向的边缘折皱被拉扁成横条。
2. **现在的做法（`600×160`，`SLICE = 32` 源 px = `16 CSS px`）**：
   - **边缘 3D 折皱 1:1 保留 + 余弦羽化过渡**：四条边直接从 `probe-sheet.png` 取带 3D 折皱与豁口的
     真实边缘段（保留 55%~78% 的边缘明暗浮雕），并在 `d = 16..31` 之间用余弦权重 `0.5*(1+cos(πt))`
     平滑羽化进内芯纹理 ⇒ 撕口处保留完整的 3D 边缘折皱，而 `d = 31..32`（`SLICE = 32` 接缝处）
     权重归零，**天然零台阶、零内嵌方框**；
   - **内芯兼具纸纹与横向纤维折皱**：内芯叠加面板净区纸纹（0.45×）与横向纤维折皱（0.65×）；
   - **保持原版窄宽度**：左右边内移 4 源 px（平均内缩 17~22 源 px），四角逐点 `min` 连续求交，
     外圈 6 源 px 与 `<20` 残影清零。
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


def norm_edge_rgb(edge_rgba: np.ndarray, keep: float) -> np.ndarray:
    rgb = edge_rgba[:, :, :3]
    a = edge_rgba[:, :, 3]
    m = a > 180
    med = np.median(rgb[m], axis=0) if m.any() else PANEL_RGB
    dev = np.clip((rgb - med) * keep, -32.0, 18.0)
    return np.clip(PANEL_RGB + dev, 0, 255)


def feather_weight(d_idx: int) -> float:
    t = float(np.clip((d_idx - 16.0) / 15.0, 0.0, 1.0))
    return float(0.5 * (1.0 + np.cos(np.pi * t)))


def build() -> None:
    if not SRC.exists():
        sys.exit(f"找不到 {SRC}")

    src = np.asarray(Image.open(SRC).convert("RGBA"), dtype=np.float32)
    sh, sw = src.shape[:2]
    if (sw, sh) != (760, 1120):
        sys.exit(f"源纸尺寸异常: {(sw, sh)} != (760, 1120)")

    # 1. 内芯：面板净区纸纹 + 横向纤维折皱
    base_crop = src[260 : 260 + OUT_H, 80 : 80 + OUT_W, :3]
    base_mean = base_crop.mean(axis=(0, 1))
    crease_src = Image.fromarray(
        np.clip(src[480 : 480 + OUT_H, 320:440, :3], 0, 255).astype(np.uint8)
    ).resize((OUT_W, OUT_H), Image.BILINEAR)
    crease_arr = np.asarray(crease_src, dtype=np.float32)
    crease_dev = crease_arr - crease_arr.mean(axis=(0, 1))
    rgb_interior = np.clip(
        PANEL_RGB + (base_crop - base_mean) * 0.45 + crease_dev * 0.65,
        PANEL_RGB - 13.0,
        PANEL_RGB + 13.0,
    )

    # 2. 四边：取源图带 3D 边缘折皱与豁口的真实边带，左右内移 4 源 px 保持窄宽度
    top_rgba = src[0:SLICE, 80 : 80 + OUT_W].copy()
    bot_rgba = src[sh - SLICE : sh, 80 : 80 + OUT_W].copy()
    left_rgba = np.zeros((OUT_H, SLICE, 4), dtype=np.float32)
    left_rgba[:, 4:SLICE, :] = src[545 : 545 + OUT_H, 0 : SLICE - 4, :]
    right_rgba = np.zeros((OUT_H, SLICE, 4), dtype=np.float32)
    right_rgba[:, 0 : SLICE - 4, :] = src[385 : 385 + OUT_H, sw - (SLICE - 4) : sw, :]

    top_rgb = norm_edge_rgb(top_rgba, 0.72)
    bot_rgb = norm_edge_rgb(bot_rgba, 0.55)
    left_rgb = norm_edge_rgb(left_rgba, 0.78)
    right_rgb = norm_edge_rgb(right_rgba, 0.78)

    # 3. 在 d=16..31 之间余弦羽化进内芯，保证撕口处有 3D 边缘折皱、SLICE=32 处零接缝
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

    # 4. Alpha：四边真撕口 + 四角连续求交 + 外沿残影清零
    alpha = np.full((OUT_H, OUT_W), 255.0, dtype=np.float32)
    a_t = top_rgba[:, :, 3]
    a_t[-1, :] = np.maximum(a_t[-1, :], 254)
    a_b = bot_rgba[:, :, 3]
    a_b[0, :] = np.maximum(a_b[0, :], 254)
    a_l = left_rgba[:, :, 3]
    a_l[:, -1] = np.maximum(a_l[:, -1], 254)
    a_r = right_rgba[:, :, 3]
    a_r[:, 0] = np.maximum(a_r[:, 0], 0)
    alpha[0:SLICE, :] = np.minimum(alpha[0:SLICE, :], a_t)
    alpha[OUT_H - SLICE : OUT_H, :] = np.minimum(alpha[OUT_H - SLICE : OUT_H, :], a_b)
    alpha[:, 0:SLICE] = np.minimum(alpha[:, 0:SLICE], a_l)
    alpha[:, OUT_W - SLICE : OUT_W] = np.minimum(alpha[:, OUT_W - SLICE : OUT_W], a_r)
    alpha[alpha < 20] = 0
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
