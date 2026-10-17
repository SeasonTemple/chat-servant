# /// script
# requires-python = ">=3.11,<3.12"
# ///
"""把 PP-OCRv6 tiny 的 ONNX 权重取到 `target/models/ppocrv6-tiny/` 并核对 SHA256。

## 为什么单独一个脚本

「tiny 权重从哪来、是不是官方那份」必须有可复现的出处，
而不是在某个 bench 脚本里手抄一条 URL。URL 与 SHA256 直接取自
**本机 venv 里 rapidocr 3.10.0 的 `default_models.yaml`**（官方模型表），
不在本文件里硬编——改版时这份表会变，硬编的 URL 会悄悄指到别的东西。

```
E:\\devSpace\\chat-servant-m31f\\target\\bench-venv\\Scripts\\python.exe scripts/ocrbench/fetch_tiny.py
```

**字典文件不重复下载**：tiny 与 small 共用同一份 `ppocrv6_dict.txt`
（同一 ocr_version / lang_type，rec 字典由 `Rec.rec_keys_path` 显式指定），
所以直接复用 `target/models/ppocrv6-small/` 下那份，已核对 SHA256。
"""

from __future__ import annotations

import hashlib
import sys
import urllib.request
from pathlib import Path

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
VENV_SITE = ROOT / "target" / "bench-venv" / "Lib" / "site-packages"
SMALL = ROOT / "target" / "models" / "ppocrv6-small"
TINY = ROOT / "target" / "models" / "ppocrv6-tiny"

# rec 字典共用，det/rec 各一个模型。key 与 default_models.yaml 里的 model_key 一致。
MODEL_KEYS = ["multi_PP-OCRv6_det_tiny", "multi_PP-OCRv6_rec_tiny"]


def model_table() -> dict:
    """从 rapidocr 自带的表里取 URL + SHA256，不在本文件里硬编。"""
    sys.path.insert(0, str(VENV_SITE))
    from omegaconf import OmegaConf

    from rapidocr.inference_engine.base import MODEL_URL_PATH

    return OmegaConf.to_container(OmegaConf.load(MODEL_URL_PATH), resolve=True)


def sha256_of(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def main() -> int:
    table = model_table()
    det = table["onnxruntime"]["PP-OCRv6"]["det"]
    rec = table["onnxruntime"]["PP-OCRv6"]["rec"]

    TINY.mkdir(parents=True, exist_ok=True)
    plan = []
    for key in MODEL_KEYS:
        src = det if key in det else rec
        entry = src[key]
        plan.append((key, entry["model_dir"], entry["SHA256"]))

    rc = 0
    for key, url, want in plan:
        name = url.rsplit("/", 1)[-1]
        dst = TINY / name
        if dst.exists():
            got = sha256_of(dst)
            print(f"{key}: 已存在 {name} ({dst.stat().st_size} B)")
        else:
            print(f"{key}: 下载 {url}", flush=True)
            tmp = dst.with_suffix(".part")
            with urllib.request.urlopen(url, timeout=180) as r, tmp.open("wb") as f:
                while chunk := r.read(1 << 20):
                    f.write(chunk)
            tmp.replace(dst)
            got = sha256_of(dst)

        ok = got == want
        print(f"  SHA256 {got}  期望 {want}  -> {'OK' if ok else 'MISMATCH'}")
        if not ok:
            rc = 1

    dict_src = SMALL / "ppocrv6_dict.txt"
    dict_dst = TINY / "ppocrv6_dict.txt"
    if dict_src.exists():
        if not dict_dst.exists():
            dict_dst.write_bytes(dict_src.read_bytes())
        got = sha256_of(dict_dst)
        print(f"字典（与 small 共用）: ppocrv6_dict.txt {got}")
        if got != sha256_of(dict_src):
            print("  字典不一致")
            rc = 1
    else:
        print(f"警告：找不到 {dict_src}，tiny 与 small 会共用 small 侧的字典")

    # 许可证出处：模型表只给 URL，许可要看仓库的 LICENSE。
    # PP-OCRv6 属 PaddleOCR，Apache-2.0；small 侧已核对过同一份出处。
    print("\n许可：PaddleOCR / PP-OCRv6 = Apache-2.0（见 docs/adr/0003-dependency-strategy.md）")
    return rc


if __name__ == "__main__":
    raise SystemExit(main())
