"""在 Python 侧复刻 Rust 的 rec 预处理，用来定位「框好但字认不准」的成因。

## 为什么需要它

Rust 与 rapidocr 用**同一份权重、同一个 det 输入**
（前景占比 1.42% vs 1.39%，几乎一致），但同一帧上 CER 差着一个数量级。
差异只能在后处理：det 给的框，以及 rec 怎么把框变成 `[1,3,48,W]`。

每次改 Rust 都要重编 release（约 4 分钟），扫参数太慢。
这里把 Rust 的 `rec_input` 逐行搬过来直接喂 ONNX，
于是「换框 / 换插值 / 换宽度 / 换归一化 / 纵向扩张」都能秒级试。

## 框从哪来

`target/probe/boxes-<来源>.txt`，每行 `x0 y0 x1 y1`（源图坐标）。
`dump_det_boxes.py` 负责从两边各导一份。

## 搬的是 Rust 的哪几行

`crates/capture/src/ocr_pp_pipeline.rs`：

```rust
let sx = bx + x * bw / out_w;   // 整数最近邻，横向
let sy = by + y * bh / h;       // 整数最近邻，纵向，h = 48
out[..] = (p[c] as f32 / 255.0 - REC_MEAN[c]) / REC_STD[c];   // v6: 只 /255
```

CTC 解码按 rapidocr 的 arch_config：18708 字 + blank(0) + space(末类) = 18710 类。
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

import numpy as np
from PIL import Image

ROOT = Path(r"E:\devSpace\chat-servant-m31f")
M = ROOT / "target" / "models" / "ppocrv6-small"
GT = ROOT / "target" / "probe" / "ocrbench" / "real-ground-truth.json"

REC_H = 48
REC_STRIDE = 8

_SESSION = None


def load_chars() -> list[str]:
    chars = list((M / "ppocrv6_dict.txt").read_text(encoding="utf-8").splitlines())
    chars.insert(0, "<blank>")
    chars.append(" ")
    return chars


def session():
    global _SESSION
    if _SESSION is None:
        import onnxruntime as ort

        _SESSION = ort.InferenceSession(
            str(M / "PP-OCRv6_rec_small.onnx"), providers=["CPUExecutionProvider"]
        )
    return _SESSION


def ctc_decode(logits: np.ndarray, chars: list[str]) -> str:
    idx = logits.argmax(axis=1)
    out, prev = [], -1
    for i in idx:
        i = int(i)
        if i != prev and i != 0:
            out.append(chars[i])
        prev = i
    return "".join(out)


def rec_input(rgb: np.ndarray, box: tuple[int, int, int, int], out_w: int,
              interp: str, norm: str) -> np.ndarray:
    x0, y0, x1, y1 = box
    fh, fw = rgb.shape[:2]
    bx, by = max(0, x0), max(0, y0)
    bw = max(1, min(x1 - x0, fw - bx))
    bh = max(1, min(y1 - y0, fh - by))
    crop = rgb[by:by + bh, bx:bx + bw].astype(np.float32)
    if interp == "nearest":
        ys = np.minimum((np.arange(REC_H) * bh) // REC_H, bh - 1)
        xs = np.minimum((np.arange(out_w) * bw) // out_w, bw - 1)
        small = crop[ys[:, None], xs[None, :]]
    else:
        import cv2

        small = cv2.resize(crop, (out_w, REC_H), interpolation=cv2.INTER_LINEAR)
    t = small.transpose(2, 0, 1) / 255.0
    if norm == "meanstd":
        t = (t - 0.5) / 0.5
    return t[np.newaxis, ...].astype(np.float32)


def pad_box(box: tuple[int, int, int, int], grow: float, fh: int) -> tuple[int, int, int, int]:
    x0, y0, x1, y1 = box
    cy = (y0 + y1) / 2.0
    half = (y1 - y0) / 2.0 * grow
    return x0, max(0, int(round(cy - half))), x1, min(fh, int(round(cy + half)))


def out_width(box: tuple[int, int, int, int], aspect: bool) -> int:
    x0, y0, x1, y1 = box
    w = (x1 - x0) * REC_H // max(1, y1 - y0) if aspect else (x1 - x0)
    return max(((w + REC_STRIDE - 1) // REC_STRIDE) * REC_STRIDE, REC_STRIDE)


def norm(s: str) -> str:
    s = "".join(chr(ord(c) - 0xFEE0) if 0xFF01 <= ord(c) <= 0xFF5E else c for c in s)
    return re.sub(r"\s+", "", s).lower()


def cer(a: str, b: str) -> tuple[int, int]:
    a, b = norm(a), norm(b)
    prev = list(range(len(b) + 1))
    for i, ca in enumerate(a, 1):
        cur = [i]
        for j, cb in enumerate(b, 1):
            cur.append(min(prev[j] + 1, cur[j - 1] + 1, prev[j - 1] + (ca != cb)))
        prev = cur
    return prev[-1], len(a)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--boxes", required=True, help="boxes-<名>.txt 里的 <名>")
    ap.add_argument("--frame", default="gws-chat")
    ap.add_argument("--aspect", choices=["yes", "no"], default="yes")
    ap.add_argument("--interp", choices=["nearest", "linear"], default="nearest")
    ap.add_argument("--norm", choices=["div255", "meanstd"], default="div255")
    ap.add_argument("--grow", type=float, default=1.0, help="纵向扩张倍数")
    ap.add_argument("--quiet", action="store_true")
    a = ap.parse_args()

    raw = (ROOT / "target" / "probe" / f"boxes-{a.boxes}.txt").read_text(encoding="utf-8")
    # ⚠ 按**行**拆：`raw.split()` 会连换行一起拆，于是每「行」只剩一个数字。
    boxes = [tuple(int(v) for v in ln.split()) for ln in raw.splitlines() if ln.strip()]

    img = np.array(
        Image.open(ROOT / "target" / "probe" / f"{a.frame}.png").convert("RGB")
    )
    boxes = [pad_box(b, a.grow, img.shape[0]) for b in boxes]

    chars = load_chars()
    s = session()
    hyp, used_w = [], []
    for b in boxes:
        w = out_width(b, a.aspect == "yes")
        buf = rec_input(img, b, w, a.interp, a.norm)
        hyp.append(ctc_decode(s.run(None, {s.get_inputs()[0].name: buf})[0][0], chars))
        used_w.append(w)

    truth = sorted(
        {f["name"]: f for f in json.loads(GT.read_text(encoding="utf-8"))["frames"]}[
            a.frame
        ]["truth"],
        key=lambda d: d["y"],
    )
    used, edits, ref = set(), 0, 0
    for t in truth:
        best, bd = None, 10**9
        for i, b in enumerate(boxes):
            if i in used:
                continue
            d = abs((b[1] + b[3]) / 2 - t["y"])
            if d < bd:
                best, bd = i, d
        if best is None:
            edits += len(norm(t["text"]))
            ref += len(norm(t["text"]))
            continue
        used.add(best)
        e, n = cer(t["text"], hyp[best])
        edits += e
        ref += n

    tag = f"{a.boxes} grow={a.grow} aspect={a.aspect} {a.interp} {a.norm}"
    print(f"[{tag}] 框 {len(boxes)}  CER {edits / max(1, ref) * 100:6.2f}%  ({edits}/{ref})")
    if not a.quiet:
        for b, w, h in zip(boxes, used_w, hyp):
            print(f"   y={b[1]:4d}..{b[3]:4d} h={b[3]-b[1]:3d} recW={w:5d} | {h}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
