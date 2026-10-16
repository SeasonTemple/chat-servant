"""把 PP-OCRv6 small 直接打到**真实微信帧**上——合成语料的 0% 漏检能不能成立。

## 为什么必须做这一步

`ocrbench` 在 60 张合成图上量到 PP-OCRv6 small 的漏检率 0.00%、CER ≈0.0005，
而路线 A（WinRT）是 19~30% 漏检。但**合成图不是微信**——
子代理已经指出合成语料与真帧存在差异，而「谁更好」这个结论只能由真帧下。

## 这不是新工具

调用形态与 `scripts/ocrbench/ocrbench.py` 的路线 B **完全一致**（同一份权重、
同一套参数、同样用最近邻放大到相同倍数），差别只有一处：
**图像换成真实抓帧**，而不是合成语料。

⚠ 输出里**不要**打印识别文本全文——那是真人聊天内容。
   这里只打印「每行的框高与中心 y」，用来数**有没有检出**，
   检出率是不需要看内容就能算的指标。

## 真值中心怎么来的

由 `measure_real_frame_font.py` 的**边缘密度**量出，并与 `xtask ocr` 打印的
`y` **逐条对齐复核过**（两者在同一坐标系，实测 293↔292、354↔348、405↔404…）。
⚠ **按帧不同**，所以必须用命令行参数传进来——
   写死一套默认值等于「拿上一帧的答案量这一帧」。
"""

from __future__ import annotations

import sys
import time
from pathlib import Path

import numpy as np
from PIL import Image

ROOT = Path(__file__).resolve().parents[2]
MODELS = ROOT / "target" / "models" / "ppocrv6-small"

# 帧 1（G8W 会话）的真值中心，1× 坐标。仅作默认值。
DEFAULT_CENTERS = [185, 241, 292, 348, 404, 460, 516]


def main() -> int:
    if len(sys.argv) < 3:
        print("用法: route_b_on_real_frame.py <帧.png> <scale> [真值中心,逗号分隔]")
        return 2
    frame = Path(sys.argv[1])
    if not frame.is_absolute():
        frame = ROOT / frame
    scale = int(sys.argv[2]) if len(sys.argv) > 2 else 2
    centers = (
        [int(v) for v in sys.argv[3].split(",")] if len(sys.argv) > 3 else DEFAULT_CENTERS
    )

    import logging

    logging.getLogger("rapidocr").setLevel(logging.ERROR)
    from rapidocr import RapidOCR

    engine = RapidOCR(
        params={
            "Det.model_path": str(MODELS / "PP-OCRv6_det_small.onnx"),
            "Rec.model_path": str(MODELS / "PP-OCRv6_rec_small.onnx"),
            "Rec.rec_keys_path": str(MODELS / "ppocrv6_dict.txt"),
        }
    )

    img = Image.open(frame).convert("RGB")
    if scale > 1:  # 最近邻，与两条路线对齐
        img = img.resize((img.width * scale, img.height * scale), Image.NEAREST)
    arr = np.array(img)[:, :, ::-1]  # rapidocr 吃 BGR

    times: list[float] = []
    boxes: list[tuple[float, float]] = []
    for _ in range(3):
        t0 = time.perf_counter()
        res = engine(arr)
        times.append((time.perf_counter() - t0) * 1000)
        # ⚠ rapidocr 3.x 的返回不是 list，是带 `.boxes` / `.txts` 的对象；
        #   而且 `.boxes` 是 numpy 数组，**不能拿它做 `or []` 的真值判断**
        #   （多元素数组的真值是模糊的，会抛 ValueError）。
        raw = getattr(res, "boxes", None)
        boxes = []
        if raw is not None and len(raw) > 0:
            for box in raw:
                ys = [p[1] for p in box]
                boxes.append((float(min(ys)) / scale, float(max(ys)) / scale))

    times.sort()
    median_ms = times[len(times) // 2]

    print(f"frame {Image.open(frame).size}  {scale}x -> {img.size}")
    print(f"time  cold {times[0]:.0f} ms  median {median_ms:.0f} ms  (3 calls)")
    print(f"detected {len(boxes)} boxes / truth {len(centers)} lines")

    hit = 0
    for c in centers:
        near = [b for b in boxes if b[0] - 8 <= c <= b[1] + 8]
        if near:
            hit += 1
            print(f"  y={c:>3}  HIT   box h {near[0][1] - near[0][0]:.0f}px")
        else:
            print(f"  y={c:>3}  MISS")
    print(f"\ndetection {hit}/{len(centers)}  miss {(len(centers) - hit) / len(centers):.1%}")
    budget = "within TICK=1000ms" if median_ms < 1000 else f"OVER by {median_ms / 1000:.2f}x"
    print(f"vs TICK=1000ms: {budget}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
