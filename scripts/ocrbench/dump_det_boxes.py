"""把两边的 det 框导成统一的 `x0 y0 x1 y1` 文本，给 emulate_rust_rec.py 用。

- `--src rapid`：走 rapidocr 的 det（读出它给的框）
- `--src rust`：解析 `xtask ocr --stats` 的逐框输出

导出格式是**源图坐标**，两边的坐标系必须一致，
否则对照出来的「框高差一倍」会是换算问题而不是后处理问题。
"""

from __future__ import annotations

import argparse
import logging
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(r"E:\devSpace\chat-servant-m31f")
M = ROOT / "target" / "models" / "ppocrv6-small"


def dump_rapid(frame: str) -> list[tuple[int, int, int, int]]:
    import numpy as np
    from PIL import Image

    logging.getLogger("rapidocr").setLevel(logging.ERROR)
    from rapidocr import RapidOCR

    img = np.array(
        Image.open(ROOT / "target" / "probe" / f"{frame}.png").convert("RGB")
    )[:, :, ::-1]
    res = RapidOCR(
        params={
            "Det.model_path": str(M / "PP-OCRv6_det_small.onnx"),
            "Rec.model_path": str(M / "PP-OCRv6_rec_small.onnx"),
            "Rec.rec_keys_path": str(M / "ppocrv6_dict.txt"),
        }
    )(img)
    out = []
    for b in res.boxes:
        xs = [p[0] for p in b]
        ys = [p[1] for p in b]
        out.append((int(round(min(xs))), int(round(min(ys))),
                    int(round(max(xs))), int(round(max(ys)))))
    return out


def dump_rust(frame: str) -> list[tuple[int, int, int, int]]:
    exe = ROOT / "target" / "release" / "xtask.exe"
    proc = subprocess.run(
        [str(exe), "ocr", f"target/probe/{frame}.png", "--scale", "1", "--stats"],
        cwd=str(ROOT), capture_output=True, text=True, encoding="utf-8", errors="replace",
        env={**__import__("os").environ,
             "CHATSERVANT_OCR": "ppocr", "CHATSERVANT_OCR_BACKEND": "ort"},
    )
    out = []
    for x, y, w, h in re.findall(r"x=\s*(\d+)\s+y=\s*(\d+)\s+w=\s*(\d+)\s+h=\s*(\d+)",
                                 proc.stdout):
        x, y, w, h = int(x), int(y), int(w), int(h)
        out.append((x, y, x + w, y + h))
    return out


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--src", choices=["rapid", "rust"], required=True)
    ap.add_argument("--frame", default="gws-chat")
    a = ap.parse_args()

    boxes = dump_rapid(a.frame) if a.src == "rapid" else dump_rust(a.frame)
    dest = ROOT / "target" / "probe" / f"boxes-{a.src}-{a.frame}.txt"
    dest.write_text("\n".join(f"{x0} {y0} {x1} {y1}" for x0, y0, x1, y1 in boxes) + "\n",
                    encoding="utf-8")
    hs = [b[3] - b[1] for b in boxes]
    print(f"{a.src}: {len(boxes)} 框 -> {dest}")
    print(f"  高 min/中位/max = {min(hs)}/{sorted(hs)[len(hs) // 2]}/{max(hs)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
