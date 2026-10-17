"""在**真实微信帧**上算字符错误率（CER）——两条路线同一把尺子。

## 为什么需要这一步

之前只量了**检出率**（有没有把行找出来），那不需要看内容。
CER 需要**标准答案**，而答案键是用户提供的真人会话内容
（存在 `target/probe/ocrbench/real-ground-truth.json`，**gitignored，不进仓库**）。

## 计分口径

- **归一化**（两边同一套）：去掉所有空白、全角转半角、ASCII 大小写统一。
  ⚠ 去掉空白这一条**偏袒 WinRT**——实测它会把英文单词里的空格吞掉
    （`OK, bringthereportv2`）。所以**同时报一版保留空白的**，
    免得主口径掩盖真实退化。
- **配对**：按 y 坐标就近配对（容差 8px）。配不上的**答案行按整行错计**
  （漏检 = 该行每个字符都算错），多出来的识别行单独计为**虚检**。
- **CER** = 编辑距离 / 答案字符数。

## 不打印识别文本

脚本只输出聚合数字与每行的**错误数**，不打印识别出来的正文——
那也是真人会话内容。

## 跑法

```
bench_real_cer.py winrt   # 需要先由 xtask ocr 产出 hyps json
bench_real_cer.py b
bench_real_cer.py both
"""

from __future__ import annotations

import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
GT = ROOT / "target" / "probe" / "ocrbench" / "real-ground-truth.json"
HYP_A = ROOT / "target" / "probe" / "ocrbench" / "winrt-hyps.json"
MODELS_B = ROOT / "target" / "models" / "ppocrv6-small"

TOL = 8  # y 配对容差


def norm(s: str, keep_space: bool) -> str:
    s = "".join(
        chr(ord(c) - 0xFEE0) if 0xFF01 <= ord(c) <= 0xFF5E else c for c in s
    )
    if not keep_space:
        s = re.sub(r"\s+", "", s)
    return s.lower()


def lev(a: str, b: str) -> int:
    if a == b:
        return 0
    prev = list(range(len(b) + 1))
    for i, ca in enumerate(a, 1):
        cur = [i]
        for j, cb in enumerate(b, 1):
            cur.append(min(prev[j] + 1, cur[j - 1] + 1, prev[j - 1] + (ca != cb)))
        prev = cur
    return prev[-1]


def score(truth: list[dict], hyps: list[dict], keep_space: bool) -> dict:
    used = [False] * len(hyps)
    edits = 0
    ref_len = 0
    matched = 0
    per_line = []
    for t in truth:
        r = norm(t["text"], keep_space)
        ref_len += len(r)
        best, bd = None, TOL + 1
        for i, h in enumerate(hyps):
            if used[i]:
                continue
            d = abs(h["y"] - t["y"])
            if d < bd:
                best, bd = i, d
        if best is None:
            per_line.append({"y": t["y"], "hit": False, "ref": len(r), "err": len(r)})
            edits += len(r)
            continue
        used[best] = True
        matched += 1
        h = norm(hyps[best]["text"], keep_space)
        e = lev(h, r)
        edits += e
        per_line.append({"y": t["y"], "hit": True, "ref": len(r), "err": e})
    spurious = [
        {"y": hyps[i]["y"], "len": len(norm(hyps[i]["text"], keep_space))}
        for i in range(len(hyps))
        if not used[i]
    ]
    return {
        "cer": edits / ref_len if ref_len else 0.0,
        "matched": matched,
        "total": len(truth),
        "ref_len": ref_len,
        "spurious": spurious,
        "per_line": per_line,
    }


def load_truth() -> list[dict]:
    return json.loads(GT.read_text(encoding="utf-8"))["frames"]


def run_b(name: str) -> list[dict]:
    import logging
    import time

    import numpy as np
    from PIL import Image

    logging.getLogger("rapidocr").setLevel(logging.ERROR)
    from rapidocr import RapidOCR

    engine = RapidOCR(
        params={
            "Det.model_path": str(MODELS_B / "PP-OCRv6_det_small.onnx"),
            "Rec.model_path": str(MODELS_B / "PP-OCRv6_rec_small.onnx"),
            "Rec.rec_keys_path": str(MODELS_B / "ppocrv6_dict.txt"),
        }
    )
    img = Image.open(ROOT / "target" / "probe" / f"{name}.png").convert("RGB")
    arr = np.array(img)[:, :, ::-1]
    res = engine(arr)
    raw = getattr(res, "boxes", None)
    txts = getattr(res, "txts", None) or []
    out = []
    if raw is not None and len(raw) > 0:
        for box, txt in zip(raw, txts):
            ys = [p[1] for p in box]
            out.append({"y": (float(min(ys)) + float(max(ys))) / 2, "text": str(txt)})
    out.sort(key=lambda d: d["y"])
    return out


def main() -> int:
    which = sys.argv[1] if len(sys.argv) > 1 else "both"
    frames = load_truth()
    if which in ("winrt", "both") and not HYP_A.exists():
        print(f"缺少 {HYP_A}——先用 xtask ocr 跑 WinRT 并存成 json")
    hyps_a = json.loads(HYP_A.read_text(encoding="utf-8")) if HYP_A.exists() else {}

    out_lines = []

    def emit(s: str = "") -> None:
        out_lines.append(s)
        print(s)

    emit("口径：去空白 / 保留空白两版都报（去空白偏袒 WinRT，它吞英文空格）")
    emit("配对容差 ±8px；配不上的答案行整行计错；多出来的识别行计虚检\n")

    totals = {"winrt": {}, "b": {}}
    for f in frames:
        name = f["name"]
        truth = sorted(f["truth"], key=lambda d: d["y"])
        emit(f"===== {name}（{len(truth)} 条答案）=====")
        for tag, hyps in (("winrt", hyps_a.get(name, [])), ("b", run_b(name) if which in ("b", "both") else [])):
            for keep, label in ((False, "去空白"), (True, "保留空白")):
                r = score(truth, hyps, keep)
                emit(
                    f"  {tag:<6} {label}  CER {r['cer']:.4f}  "
                    f"配对 {r['matched']}/{r['total']}  答案字符 {r['ref_len']}  "
                    f"虚检 {len(r['spurious'])}"
                )
                if not keep:
                    totals[tag][name] = r
        emit()

    emit("===== 合计（去空白口径）=====")
    for tag in ("winrt", "b"):
        frames_ok = [v for k, v in totals[tag].items()]
        if not frames_ok:
            continue
        e = sum(x["edits"] if "edits" in x else 0 for x in frames_ok)  # 占位，下面重算
        edits = 0
        ref = 0
        for x in frames_ok:
            for pl in x["per_line"]:
                edits += pl["err"]
                ref += pl["ref"]
        m = sum(x["matched"] for x in frames_ok)
        t = sum(x["total"] for x in frames_ok)
        emit(f"  {tag:<6} CER {edits / ref:.4f}  配对 {m}/{t}  虚检 {sum(len(x['spurious']) for x in frames_ok)}")

    dest = ROOT / "target" / "probe" / "ocrbench" / "real-cer-report.txt"
    dest.write_text("\n".join(out_lines), encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
