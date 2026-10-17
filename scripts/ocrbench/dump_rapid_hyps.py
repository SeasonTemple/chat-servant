"""把参照实现 rapidocr 的识别行导出成 cer_real_frame.py 能读的同一份格式。

## 为什么需要这一步

之前的两份 CER 数字（rapidocr 0.79% vs Rust 2.38%）**不是同一个东西**，
问题和 ADR 第（四）节里记的「0.79% 与 18.60%」是同一类，换了个方向而已：

| | 算 CER 的脚本 | y 的定义 | 配对容差 |
|---|---|---|---|
| rapidocr 0.79% | `bench_real_cer.py` | 框**中心** `(min+max)/2` | ±8px |
| Rust 2.38% | `cer_real_frame.py` | 框**顶部** `rect.y0`（`xtask/src/main.rs:1047`） | ±30px |

两把不同的尺子量出来的数，放进同一张表对比，是没有意义的。
⇒ 这里把参照实现也导成**框顶部**、交给 `cer_real_frame.py` 用**同一段
配对代码**去算，差值才是可比的。

## 关键：构造参数必须与 bench_real_cer.run_b 逐字一致

det 的 limit_side_len / 阈值一旦不同，框就不是同一批框，比出来的字准差
也归因不到代码。所以 RapidOCR 的构造**从 bench_real_cer 抄**，不重写。
"""

from __future__ import annotations

import logging
import sys
from pathlib import Path

ROOT = Path(r"E:\devSpace\chat-servant-m31f")
sys.path.insert(0, str(ROOT / "scripts" / "ocrbench"))

# 从 bench_real_cer 拿同一份构造方式与同一份答案键路径，不重写一遍。
import bench_real_cer as ref  # noqa: E402

OUT = ROOT / "target" / "probe" / "cer2"
FRAMES = ["wechat-chat", "gws-chat"]


def build():
    """RapidOCR 实例，参数与 bench_real_cer.run_b 逐字相同。"""
    from rapidocr import RapidOCR

    m = ref.MODELS_B
    return RapidOCR(
        params={
            "Det.model_path": str(m / "PP-OCRv6_det_small.onnx"),
            "Rec.model_path": str(m / "PP-OCRv6_rec_small.onnx"),
            "Rec.rec_keys_path": str(m / "ppocrv6_dict.txt"),
        }
    )


def main() -> int:
    import numpy as np
    from PIL import Image

    logging.getLogger("rapidocr").setLevel(logging.ERROR)
    engine = build()
    OUT.mkdir(parents=True, exist_ok=True)

    for name in FRAMES:
        img = Image.open(ROOT / "target" / "probe" / f"{name}.png").convert("RGB")
        arr = np.array(img)[:, :, ::-1]
        res = engine(arr)
        boxes = getattr(res, "boxes", None)
        txts = getattr(res, "txts", None) or []
        rows: list[tuple[int, int, str]] = []
        if boxes is not None and len(boxes) > 0:
            for box, txt in zip(boxes, txts):
                ys = [float(p[1]) for p in box]
                # 顶部 + 高度，与 xtask 打印的 `y=rect.y0 h=rect.height()` 同口径。
                rows.append((int(round(min(ys))), int(round(max(ys) - min(ys))), str(txt)))
        rows.sort(key=lambda r: r[0])

        # 量具自检：框坐标全 0 或行数明显异常，说明读的不是像素坐标，
        # 后面 CER 会静默变成「全漏检」，那时数字是假的而不是差。
        assert rows, f"{name}: 参照实现一行都没出，dump 不可信"
        assert all(r[1] > 0 for r in rows), f"{name}: 出现高度为 0 的框：{rows}"
        assert max(r[0] for r in rows) > 100, f"{name}: y 全在 100 以内，像素坐标可疑：{rows}"

        lines = [
            f"{i:>3} | y={y:>4} h={h:>2} | {t}"
            for i, (y, h, t) in enumerate(rows)
        ]
        dest = OUT / f"{name}-rapid.txt"
        dest.write_text("\n".join(lines) + "\n", encoding="utf-8")
        print(f"{name}: {len(rows)} 行 -> {dest}")
        for ln in lines:
            print("   " + ln)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
