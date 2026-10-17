"""det 侧的决定性 A/B：Rust「不放大、只补边」vs rapidocr「真放大」。

## 假设

Rust 的 `det_size_with_min_side()` 算出 (1184, 736)（855x545 的帧，min_side=736），
但 `det_input()` **不缩放**，只把原图贴左上角、右下用「复制最后一行/列」补齐：

```rust
let sy = y.min(fh.saturating_sub(1));
let sx = x.min(fw.saturating_sub(1));
```

⇒ `det_min_side = 736` 对**像素内容**毫无作用，det 看到的是原生分辨率的小字。
rapidocr 的 `DetPreProcess.resize` 则真的 `cv2.resize` 放大 1.35 倍。

## 这个脚本怎么同构地模拟 Rust

只改两处，其余一字不动：

1. 喂给 OCR 的图先按 Rust 的规则补边到 (det_w, det_h)（复制最后一行/列）；
2. `Det.limit_side_len = 0` ⇒ rapidocr 不再缩放，pred 与 ori 同尺寸 ⇒ **框映射 1:1**，
   与 Rust `det_boxes()` 的 `det_w / pw` 恒等情形一致。

rec 那边从同一张（补边后的）图裁，而内容区与原图逐像素相同，所以裁剪结果不变。

## 判据

`baseline` 0.79% → `pad` 若掉到 18% 附近 ⇒ **缺的那次放大就是破口**。
两档都不动 ⇒ 这个假设被否，得继续往别处找（别硬套）。
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

import numpy as np

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts" / "ocrbench"))

GT = ROOT / "target" / "probe" / "ocrbench" / "real-ground-truth.json"
MODELS = ROOT / "target" / "models" / "ppocrv6-small"

DET_STRIDE = 32
MIN_SIDE = 736


def rust_det_size(w: int, h: int) -> tuple[int, int]:
    """crates/capture/src/ocr_pp_pipeline.rs 的 det_size_with_min_side 逐字搬过来。"""
    fw, fh = float(w), float(h)
    short = min(fw, fh)
    if MIN_SIDE > 0 and 0 < short < MIN_SIDE:
        ratio = MIN_SIDE / short
        fw *= ratio
        fh *= ratio
    fw = int(np.ceil(round(fw) / DET_STRIDE) * DET_STRIDE)
    fh = int(np.ceil(round(fh) / DET_STRIDE) * DET_STRIDE)
    return max(fw, DET_STRIDE), max(fh, DET_STRIDE)


def rust_pad(img: np.ndarray, w: int, h: int) -> np.ndarray:
    """det_input 的补边语义：越界取 min(x, fw-1) / min(y, fh-1)，即复制边缘。"""
    fh, fw = img.shape[:2]
    ys = np.minimum(np.arange(h), fh - 1)
    xs = np.minimum(np.arange(w), fw - 1)
    return img[ys[:, None], xs[None, :]]


def run(name: str, mode: str) -> list[dict]:
    import logging

    logging.getLogger("rapidocr").setLevel(logging.ERROR)
    from PIL import Image
    from rapidocr import RapidOCR

    img = np.array(Image.open(ROOT / "target" / "probe" / f"{name}.png").convert("RGB"))
    img = img[:, :, ::-1]  # RGB -> BGR，和 bench_real_cer.run_b 一致

    params = {
        "Det.model_path": str(MODELS / "PP-OCRv6_det_small.onnx"),
        "Rec.model_path": str(MODELS / "PP-OCRv6_rec_small.onnx"),
        "Rec.rec_keys_path": str(MODELS / "ppocrv6_dict.txt"),
    }
    if mode == "pad":
        dh, dw = rust_det_size(img.shape[1], img.shape[0])
        print(f"  [{name}] 原图 {img.shape[1]}x{img.shape[0]} -> Rust 补边到 {dw}x{dh}")
        img = rust_pad(img, dw, dh)
        params["Det.limit_side_len"] = 0  # 不再缩放 => 框映射 1:1

    res = RapidOCR(params=params)(img)
    boxes = getattr(res, "boxes", None)
    txts = getattr(res, "txts", None) or []
    out = []
    if boxes is not None and len(boxes) > 0:
        for box, txt in zip(boxes, txts):
            ys = [p[1] for p in box]
            out.append({"y": (float(min(ys)) + float(max(ys))) / 2, "text": str(txt)})
    out.sort(key=lambda d: d["y"])
    return out


def main() -> int:
    mode = "baseline"
    for a in sys.argv[1:]:
        if a.startswith("--"):
            mode = a[2:]
    print(f">>> mode = {mode}")

    import bench_real_cer as B

    gt = json.loads(GT.read_text(encoding="utf-8"))
    total_e = total_r = matched = total = 0
    for f in gt["frames"]:
        truth = sorted(f["truth"], key=lambda d: d["y"])
        hyps = run(f["name"], mode)
        r = B.score(truth, hyps, keep_space=False)
        total_e += sum(p["err"] for p in r["per_line"])
        total_r += sum(p["ref"] for p in r["per_line"])
        matched += r["matched"]
        total += r["total"]
        print(f"  {f['name']:<12} CER {r['cer']*100:6.2f}%  配对 {r['matched']}/{r['total']}"
              f"  读出 {len(hyps)} 行  虚检 {len(r['spurious'])}")
    cer = total_e / total_r if total_r else 0.0
    print(f"\n[{mode}] 合计 CER {cer*100:.2f}%  配对 {matched}/{total}"
          f"  (编辑 {total_e}/{total_r})")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
