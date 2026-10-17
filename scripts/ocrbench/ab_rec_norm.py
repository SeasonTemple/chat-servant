"""rec 前处理的 A/B：Rust 侧与 rapidocr 之间到底差在哪一个变量。

## 背景

同一份 ONNX 权重、同一批真帧、同一台机器：

| 路线 | CER | 配对 |
|---|---|---|
| Python rapidocr 3.10.0 | 0.79% | 17/17 |
| Rust（xtask ocr，两后端同结果） | 18.60% | 17/17 |

⇒ 差的不是权重、不是检出（两边都 17/17），是**喂进去的那张图**。
两个后端（tract / ort）结果逐字相同 ⇒ 也不是推理后端。

## 本脚本测什么

按 `--mode` 猴补丁 rapidocr 的 `resize_norm_img`，其它一字不动：

- `baseline` 原样
- `norm255`  只 `x/255`，去掉 `-=0.5 / /=0.5`
- `nearest` rec 裁剪缩放改成 `INTER_NEAREST`（rapidocr 默认 INTER_LINEAR）
- `both`    两个都改

哪一档把 0.79% 打到 18% 附近，那个变量就是破口；全都不动则两个都被否，
得往 det 侧或裁剪坐标那边找。

⚠ **量具自检**：每次都打「猴补丁被调用 N 次」。0 = 补丁没生效，
此时「改了没变化」会被误读成「这个变量无所谓」——
这个坑已经踩过一次（第一版没打计数，差点把否证当成证据）。
"""

from __future__ import annotations

import json
import math
import sys
from pathlib import Path

import numpy as np

# ⚠ Windows 上 Python 的 stdout 默认是传统代码页（GitHub 的 Windows runner
#   上就是 cp1252），一句中文 print 就 UnicodeEncodeError 带栈回溯崩掉。
#   本仓在 scripts/fetch_models.py 上踩过（CI run 39037571268），
#   而 ocrbench 这一族当时**一个都没加**——守卫的不变量只管
#   「gate.ps1 会 python 调起的脚本」，人工跑的台架在射程外。
#   不能指望调用方设 PYTHONIOENCODING，必须在脚本里自己改。
#   errors="replace"：真有编不出来的字符时**降级成 ? 而不是崩**——
#   台架的失败模式必须是「读数不符」，不是「打印失败 ⇒ 栈回溯」。
for _stream in (sys.stdout, sys.stderr):
    if hasattr(_stream, "reconfigure"):        # 3.7+ 都有
        _stream.reconfigure(encoding="utf-8", errors="replace")

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts" / "ocrbench"))

GT = ROOT / "target" / "probe" / "ocrbench" / "real-ground-truth.json"

CALLS = {"norm": 0, "resize": 0}


def apply(mode: str) -> None:
    import cv2
    from rapidocr.ch_ppocr_rec import main as recmain

    if mode in ("norm255", "both"):
        orig = recmain.TextRecognizer.resize_norm_img

        def resize_norm_img(self, img, max_wh_ratio):
            CALLS["norm"] += 1
            img_channel, img_height, img_width = self.rec_image_shape[:3]
            img_width = int(img_height * max_wh_ratio)
            h, w = img.shape[:2]
            ratio = w / float(h)
            if math.ceil(img_height * ratio) > img_width:
                resized_w = img_width
            else:
                resized_w = int(math.ceil(img_height * ratio))
            resized = cv2.resize(img, (resized_w, img_height))
            resized = resized.astype("float32")
            # 与原实现的唯一差别：少了 -= 0.5 / /= 0.5
            resized = resized.transpose((2, 0, 1)) / 255
            pad = np.zeros((img_channel, img_height, img_width), dtype=np.float32)
            pad[:, :, 0:resized_w] = resized
            return pad

        recmain.TextRecognizer.resize_norm_img = resize_norm_img
        assert orig is not recmain.TextRecognizer.resize_norm_img

    if mode in ("nearest", "both"):
        real_resize = cv2.resize

        class _CV2Nearest:
            """只把 rec 的 resize 换成最近邻，其它属性照旧转给真 cv2。"""

            INTER_NEAREST = cv2.INTER_NEAREST
            INTER_LINEAR = cv2.INTER_LINEAR

            def __getattr__(self, name):
                return getattr(cv2, name)

            @staticmethod
            def resize(src, dsize, *a, **kw):
                CALLS["resize"] += 1
                kw["interpolation"] = cv2.INTER_NEAREST
                return real_resize(src, dsize, *a, **kw)

        recmain.cv2 = _CV2Nearest()


def main() -> int:
    mode = "baseline"
    for a in sys.argv[1:]:
        if a.startswith("--"):
            mode = a[2:]

    import bench_real_cer as B

    apply(mode)
    print(f">>> mode = {mode}  补丁已装（norm={mode in ('norm255','both')}, "
          f"nearest={mode in ('nearest','both')}）")

    gt = json.loads(GT.read_text(encoding="utf-8"))
    total_e = total_r = matched = total = 0
    for f in gt["frames"]:
        truth = sorted(f["truth"], key=lambda d: d["y"])
        hyps = B.run_b(f["name"])
        r = B.score(truth, hyps, keep_space=False)
        total_e += sum(p["err"] for p in r["per_line"])
        total_r += sum(p["ref"] for p in r["per_line"])
        matched += r["matched"]
        total += r["total"]
        print(f"  {f['name']:<12} CER {r['cer']*100:6.2f}%  配对 {r['matched']}/{r['total']}"
              f"  虚检 {len(r['spurious'])}")
    cer = total_e / total_r if total_r else 0.0
    print(f"\n[{mode}] 合计 CER {cer*100:.2f}%  配对 {matched}/{total}"
          f"  (编辑 {total_e}/{total_r})")
    print(f">>> 猴补丁被调用：norm={CALLS['norm']} resize={CALLS['resize']}"
          f"（都是 0 = 补丁没生效，别把「没变化」当证据）")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
