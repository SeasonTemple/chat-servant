"""把**生产要用的那份** PP-OCRv6 small 权重取到 `target/models/ppocrv6-small/` 并核对 SHA256。

## 为什么单独一个脚本

前面几轮里，权重是「从本机 rapidocr 的 wheel 里复制」来的——
那对做台架够用（同一个模型），但**装机场景根本走不通**：
CI 上没有 wheel，安装包也带不出 30MB 的 ONNX。
所以「正式取权重」这件事**从来没有被脚本化过**，
而它恰恰是「换默认引擎」的唯一障碍（见 ADR-0006）。

## URL 与 SHA256 不在本文件里硬编

直接复用 rapidocr 自带的模型表
（`rapidocr.inference_engine.base.MODEL_URL_PATH`）。
硬编 URL 的坏处是**它会悄悄指到别的东西**——
上游换版本时你不会收到任何通知，只会在某天装出一个来路不明的权重。
表里有 sha256，取完当场核对，对不上就报错退出。

## 用法

    target\\bench-venv\\Scripts\\python.exe scripts\\fetch_models.py
    target\\bench-venv\\Scripts\\python.exe scripts\\fetch_models.py --check

`--check` 只核对不下载，供门禁与 CI 判断「权重在不在且对不对」。
"""

from __future__ import annotations

import argparse
import hashlib
import shutil
import sys
import urllib.request
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
DEST = REPO / "target" / "models" / "ppocrv6-small"

# `ModelSet` 里那三个文件名，逐字对应 crates/capture/src/ocr_pp_pipeline.rs。
# ⚠ 改这里必须同时改那边的 ModelSet，否则产品去读一个不存在的文件。
FILES = [
    # (rapidocr 模型表里的键, 落到本地的文件名)
    # ⚠ 表里的键带 `multi_` 前缀，且挂在 `onnxruntime -> PP-OCRv6 -> det/rec` 下面。
    #   「为什么这个键和落地的文件名不一样」是这张表的历史包袱，不是我漏了前缀。
    ("multi_PP-OCRv6_det_small", "PP-OCRv6_det_small.onnx"),
    ("multi_PP-OCRv6_rec_small", "PP-OCRv6_rec_small.onnx"),
]

# 词典不在模型表里（它是 rec 的附属文件），但**必须**跟 rec 一起取，
# 否则产品会在 CTC 解码那一步才发现类数对不上。
DICT_FILE = "ppocrv6_dict.txt"


def model_table() -> dict:
    """rapidocr 自带的模型表，取其中 onnxruntime 那一支。"""
    from rapidocr.inference_engine.base import MODEL_URL_PATH
    from omegaconf import OmegaConf

    return OmegaConf.to_container(OmegaConf.load(MODEL_URL_PATH), resolve=True)


def find_entry(table: dict, key: str) -> tuple[str, str] | None:
    """在模型表里按名字找 `(url, sha256)`。

    ⚠ 表的形状是 `onnxruntime -> 大版本 -> det/rec -> 文件名 -> {model_dir, SHA256}`，
    **不是** `{文件名: {url, sha256}}`——第一版按后者写，拿到的全是 `KeyError`。
    这里不写死层数，而是**递归找**：上游加一层版本目录不该让这个脚本炸掉。
    """
    if not isinstance(table, dict):
        return None
    if key in table:
        node = table[key]
        if isinstance(node, dict) and "model_dir" in node:
            return node["model_dir"], node.get("SHA256", "")
    for v in table.values():
        if isinstance(v, dict):
            got = find_entry(v, key)
            if got is not None:
                return got
    return None


def sha256_of(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--check", action="store_true", help="只核对，不下载")
    args = ap.parse_args()

    try:
        table = model_table()
    except Exception as e:  # noqa: BLE001 - 缺 rapidocr 时要能说清缺什么
        print(f"取不到 rapidocr 的模型表：{e}", file=sys.stderr)
        print("本脚本刻意不在本地硬编 URL/SHA256 —— 请先装 rapidocr：", file=sys.stderr)
        print("    uv run --with rapidocr --with omegaconf python scripts/fetch_models.py", file=sys.stderr)
        return 2

    local_names = [name for _, name in FILES] + [DICT_FILE]

    if args.check:
        missing = [n for n in local_names if not (DEST / n).is_file()]
        if missing:
            print(f"缺：{missing}（在 {DEST}）", file=sys.stderr)
            return 1
        print(f"权重齐全：{DEST}")
        return 0

    DEST.mkdir(parents=True, exist_ok=True)
    for key, name in FILES:
        entry = find_entry(table, key)
        if entry is None:
            print(f"模型表里没有 {key}", file=sys.stderr)
            return 2
        url, want = entry
        out = DEST / name
        if out.is_file() and sha256_of(out) == want:
            print(f"已存在且校验通过  {name}")
            continue
        print(f"下载 {name} …")
        with urllib.request.urlopen(url) as r, out.open("wb") as f:
            shutil.copyfileobj(r, f)
        got = sha256_of(out)
        if got != want:
            out.unlink(missing_ok=True)
            print(
                f"SHA256 不符，已删除：{name}\n  期望 {want}\n  实得 {got}",
                file=sys.stderr,
            )
            return 1
        print(f"  校验通过 {got[:16]}…  {out.stat().st_size / 1e6:.1f} MB")

    # 词典随 wheel 一起装，但**不在模型表里**，所以这里不校验 SHA256——
    # 校验不了就别装作校验了。缺了它，rec 会在 CTC 解码那一步报「类数对不上」。
    dict_path = DEST / DICT_FILE
    if dict_path.is_file():
        lines = len(dict_path.read_text(encoding="utf-8").splitlines())
        print(f"词典已在：{DICT_FILE}（{lines} 行）")
        if lines != 18708:
            print(
                f"⚠ 词典是 {lines} 行，本项目按 18708 行核对（+blank+space = 18710 类）。"
                f"不匹配的话 rec 会静默返回空串。",
                file=sys.stderr,
            )
            return 1
    else:
        print(
            f"⚠ 缺 {DICT_FILE}。它不在 rapidocr 的模型表里，只能从本机 wheel 复制：\n"
            f"    python -c \"import rapidocr,shutil,os;"
            f"print(os.path.dirname(rapidocr.__file__))\"",
            file=sys.stderr,
        )
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
