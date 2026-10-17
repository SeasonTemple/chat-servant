"""复核 tiny 在 wechat-chat 上的 6/7：**只报 y 坐标与框高，不报文本**。

检出率是不看内容就能数的指标；这条复核的意义在于 6/7 与 7/7 是本轮
「tiny 值不值」的唯一决定性数据，不能是偶发。

跑 3 遍，每遍都报哪些 y 命中、哪些没命中。
"""
import logging
import statistics
import sys
import time
from pathlib import Path

ROOT = Path(r"E:\devSpace\chat-servant-m31f")
sys.path.insert(0, str(ROOT / "scripts" / "ocrbench"))

import numpy as np
from PIL import Image

logging.getLogger("rapidocr").setLevel(logging.ERROR)
from rapidocr import RapidOCR

from latency_breakdown import FRAMES, MODEL_DIRS, MODEL_FILES

spec = FRAMES["wechat-chat"]
arr = np.array(Image.open(spec["path"]).convert("RGB"))[:, :, ::-1]

for model in ("tiny", "small"):
    det_name, rec_name = MODEL_FILES[model]
    mdir = MODEL_DIRS[model]
    eng = RapidOCR(params={
        "Det.model_path": str(mdir / det_name),
        "Rec.model_path": str(mdir / rec_name),
        "Rec.rec_keys_path": str(mdir / "ppocrv6_dict.txt"),
    })
    for _ in range(2):
        eng(arr)
    print(f"\n=== {model}  真值中心 {spec['centers']}")
    for run in range(3):
        t = time.perf_counter()
        res = eng(arr)
        ms = (time.perf_counter() - t) * 1000
        raw = getattr(res, "boxes", None)
        spans = []
        if raw is not None and len(raw) > 0:
            for b in raw:
                yy = [p[1] for p in b]
                spans.append((round(min(yy)), round(max(yy)), round(max(yy) - min(yy))))
        spans.sort()
        hits, miss = [], []
        for c in spec["centers"]:
            near = [s for s in spans if s[0] - 8 <= c <= s[1] + 8]
            (hits if near else miss).append(c)
        print(f"  第{run+1}遍 {ms:6.0f} ms  框 {len(spans)}  检出 {len(hits)}/{len(spec['centers'])}"
              f"  MISS y={miss}")
        if run == 0:
            print(f"    框(y0,y1,h) = {spans}")
    time.sleep(3)
