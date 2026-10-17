"""补上性能实验里最要紧的两个缺口。

## 背景

`docs/adr/0007-ocr-latency-breakdown.md` 留下三条没做的：

1. **tiny 不值**——真帧上 tiny **稳定漏掉一条**（7 条里只检出 6 条），
   而那是 **det 灵敏度**的问题（框清单显示它少给一个框）。
   ⇒ 那就有个明显没试过的组合：**det 用 small（保检出）+ rec 用 tiny（换速度）**。
   rec 占了端到端的 34~57%，换掉它是最有可能「拿到速度又不丢检出」的一枪。
2. **`use_cls: false`** 没测。cls 在端到端里只占 7~8ms/帧（纯推理），
   但它要**多建一个 ONNX 会话**（冷启动 +102ms）。
   聊天是横排文本，方向分类器有没有用值得实测而不是推断。
3. **线程数固定在 4**（ADR-0007 实测的最优值），别再用默认。

## 三个候选，各跑两帧真实帧

| 组合 | det | rec | cls |
|---|---|---|---|
| A 基线 | small | small | 开 |
| B 混搭 | **small** | **tiny** | 开 |
| C 关 cls | small | small | **关** |

## 判据

- **检出**：与 `real-ground-truth.json` 的答案中心配对（±8px），报 n/N。
- **耗时**：预热后 ≥8 次交错采样，报中位数与 P95。
- ⚠ 本机有常驻动画壁纸/Chrome/微信，**噪声很大**；所以每个组合都报噪声幅度，
  且**交错采样**，不要一个组合跑完再跑下一个。

⚠ **不打印识别文本**——那是真人会话内容。只打印计数与计时。
"""

from __future__ import annotations

import json
import statistics
import sys
import time
from pathlib import Path

import numpy as np
from PIL import Image

ROOT = Path(__file__).resolve().parents[2]
PROBE = ROOT / "target" / "probe" / "ocrbench"
GT = json.loads((PROBE / "real-ground-truth.json").read_text(encoding="utf-8"))
FRAMES = {f["name"]: sorted(f["truth"], key=lambda d: d["y"]) for f in GT["frames"]}

SMALL = ROOT / "target" / "models" / "ppocrv6-small"
TINY = ROOT / "target" / "models" / "ppocrv6-tiny"
THREADS = 4
REPEAT = 8
TOL = 8

COMBOS = {
    "A  small+small+cls": dict(
        Det={"model_path": str(SMALL / "PP-OCRv6_det_small.onnx")},
        Rec={"model_path": str(SMALL / "PP-OCRv6_rec_small.onnx"),
             "rec_keys_path": str(SMALL / "ppocrv6_dict.txt")},
        use_cls=True,
    ),
    "B  small+tiny+cls": dict(
        Det={"model_path": str(SMALL / "PP-OCRv6_det_small.onnx")},
        Rec={"model_path": str(TINY / "PP-OCRv6_rec_tiny.onnx"),
             "rec_keys_path": str(TINY / "ppocrv6_dict.txt")},
        use_cls=True,
    ),
    "C  small+small-nocls": dict(
        Det={"model_path": str(SMALL / "PP-OCRv6_det_small.onnx")},
        Rec={"model_path": str(SMALL / "PP-OCRv6_rec_small.onnx"),
             "rec_keys_path": str(SMALL / "ppocrv6_dict.txt")},
        use_cls=False,
    ),
}


def build(combo: dict):
    import logging

    logging.getLogger("rapidocr").setLevel(logging.ERROR)
    from rapidocr import RapidOCR

    # ⚠ 键名必须是 `EngineConfig.onnxruntime.*`——写 `Global.*` 会被 rapidocr
    # 直接拒掉（`ValueError: Global.intra_op_num_threads is not a valid key.`），
    # 而且它是**第一次跑就崩**，不会静默用默认值。
    params = {
        "EngineConfig.onnxruntime.intra_op_num_threads": THREADS,
        "Global.use_cls": combo["use_cls"],
    }
    params.update({f"Det.{k}": v for k, v in combo["Det"].items()})
    params.update({f"Rec.{k}": v for k, v in combo["Rec"].items()})
    return RapidOCR(params=params)


def centers(res) -> list[float]:
    raw = getattr(res, "boxes", None)
    if raw is None or len(raw) == 0:
        return []
    out = []
    for box in raw:
        ys = [p[1] for p in box]
        out.append((float(min(ys)) + float(max(ys))) / 2)
    return sorted(out)


def hit_ratio(hyps: list[float], truth: list[dict]) -> tuple[int, int]:
    used = [False] * len(hyps)
    hit = 0
    for t in truth:
        best, bd = None, TOL + 1
        for i, y in enumerate(hyps):
            if used[i]:
                continue
            d = abs(y - t["y"])
            if d < bd:
                best, bd = i, d
        if best is not None:
            used[best] = True
            hit += 1
    return hit, len(truth)


def main() -> int:
    out: list[str] = []

    def emit(s: str = "") -> None:
        out.append(s)
        print(s.encode("ascii", "replace").decode("ascii"))

    imgs = {}
    for name in FRAMES:
        im = Image.open(ROOT / "target" / "probe" / f"{name}.png").convert("RGB")
        imgs[name] = np.array(im)[:, :, ::-1]

    # 每个组合建一次引擎；三个引擎先全部建好，再交错采样——避免
    # 「先跑完 A 再跑 B」把 CPU 温度/后台负载的漂移算到某个组合头上。
    engines = {label: build(c) for label, c in COMBOS.items()}

    times: dict[tuple[str, str], list[float]] = {}
    hits: dict[tuple[str, str], tuple[int, int]] = {}
    for label, eng in engines.items():
        for name, arr in imgs.items():
            for _ in range(2):  # 预热
                centers(eng(arr))
            ms = []
            for _ in range(REPEAT):
                t0 = time.perf_counter()
                r = eng(arr)
                ms.append((time.perf_counter() - t0) * 1000)
                hits[(label, name)] = hit_ratio(centers(r), FRAMES[name])
            times[(label, name)] = ms

    emit(f"线程={THREADS}  采样={REPEAT} 次（已预热）  两帧交错")
    emit(f"{'组合':<24}{'帧':<14}{'检出':<10}{'中位 ms':>9}{'P95 ms':>9}{'噪声幅度':>10}")
    for label in COMBOS:
        for name in imgs:
            ms = sorted(times[(label, name)])
            med = statistics.median(ms)
            p95 = ms[min(len(ms) - 1, int(len(ms) * 0.95))]
            spread = (p95 - ms[0]) / med * 100 if med else 0
            h, n = hits[(label, name)]
            emit(f"{label:<24}{name:<14}{f'{h}/{n}':<10}{med:>9.0f}{p95:>9.0f}{spread:>9.0f}%")
        med_all = statistics.median(
            [statistics.median(times[(label, n)]) for n in imgs]
        )
        det = " ".join(f"{n}:{hits[(label, n)][0]}/{hits[(label, n)][1]}" for n in imgs)
        emit(f"{'':<24}{'合计':<14}{det:<10}{med_all:>9.0f}")
        emit()

    emit("对 TICK=1000ms：")
    for label in COMBOS:
        for name in imgs:
            med = statistics.median(times[(label, name)])
            emit(f"  {label:<24}{name:<14}{med:>6.0f} ms  "
                 f"{'在预算内，余量 ' + str(round((1000 - med) / 10)) + '%' if med < 1000 else '超 ' + f'{med / 1000:.2f}' + ' 倍'}")

    dest = PROBE / "hybrid-compare.txt"
    dest.write_text("\n".join(out), encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
