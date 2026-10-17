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

## 词典（`ppocrv6_dict.txt`）为什么单独处理

词典**不在 `onnxruntime/PP-OCRv6/rec` 那一支里**——实测那一支的每个节点只有
`model_dir` 与 `SHA256`。`dict_url` 只挂在 `mnn` / `paddle` / `torch` 三支上，
而三支指向**同一个 URL**（`.../paddle/PP-OCRv6/rec/PP-OCRv6_rec_small/ppocrv6_dict.txt`）。

⚠ **上游没给词典的 SHA256**：`dict_url` 是个光秃秃的字符串，同一个节点里
只有 .mnn 文件的 `SHA256`，管不到词典。⇒ 本地钉一个。
钉它与本文件「URL 不硬编」的立场**不冲突**，两件事各管一头：

| | 从哪来 | 挡的是 |
|---|---|---|
| URL | 上游模型表 | 上游换版本/换位置时我们跟得上 |
| SHA256 | 本文件钉死（2026-10-18 实测） | 上游**静默**换内容时我们会响 |

上游合法更新词典时，这里会响并要求人复核——这正是想要的行为，
而不是悄悄装一个来路不明的词典。

## 用法

    target\\bench-venv\\Scripts\\python.exe scripts\\fetch_models.py
    target\\bench-venv\\Scripts\\python.exe scripts\\fetch_models.py --check

`--check` 做两件事，**都不需要权重在场**：
1. 本文件的文件名清单与 Rust 侧 `ModelSet` 逐个对齐（改一边忘另一边 ⇒ 红）；
2. 权重在场时再核对齐全性与 SHA256。
"""

from __future__ import annotations

import argparse
import hashlib
import re
import shutil
import sys
import urllib.request
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
DEST = REPO / "target" / "models" / "ppocrv6-small"
MODEL_SET_SRC = REPO / "crates" / "capture" / "src" / "ocr_pp_pipeline.rs"

# `ModelSet` 里那三个文件名，逐字对应 crates/capture/src/ocr_pp_pipeline.rs。
# ⚠ 改这里必须同时改那边的 ModelSet，否则产品去读一个不存在的文件。
FILES = [
    # (rapidocr 模型表里的键, 落到本地的文件名)
    # ⚠ 表里的键带 `multi_` 前缀，且挂在 `onnxruntime -> PP-OCRv6 -> det/rec` 下面。
    #   「为什么这个键和落地的文件名不一样」是这张表的历史包袱，不是我漏了前缀。
    ("multi_PP-OCRv6_det_small", "PP-OCRv6_det_small.onnx"),
    ("multi_PP-OCRv6_rec_small", "PP-OCRv6_rec_small.onnx"),
]

# 词典不在**我们用的那一支**模型表里（见模块 docstring），但它的 URL
# 挂在 rec 的同一个键下面，只是换了一支 backend。
DICT_KEY = "multi_PP-OCRv6_rec_small"
DICT_FILE = "ppocrv6_dict.txt"

# 上游不给词典的校验值，这个是本地实测的（2026-10-18，从模型表那个 URL 取回后算的）。
# `target/probe/dict_probe.py` 是当次的调研脚本：9 条 URL 取回 9 份，
# 其中 rec_small / rec_medium 与本机在用的那份**逐字节相同**（rec_tiny 是 6904 行，不同文件）。
DICT_SHA256 = "b5f2bfe2bdd9448429e3e82b51c789775d9b42f2403d082b00662eb77e401c5d"
DICT_LINES = 18708



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


def find_dict_url(table: dict, key: str) -> str | None:
    """找挂在同一个键下的 `dict_url`。

    ⚠ **不能靠 `find_entry`**：它要的是「带 model_dir 的那个节点」，
    而我们用的 `onnxruntime` 那一支的节点**没有 dict_url**——按它找会得到 None，
    然后非常像「上游没有词典」。真相是词典在**别的 backend 那一支**上。
    """
    if not isinstance(table, dict):
        return None
    node = table.get(key)
    if isinstance(node, dict) and isinstance(node.get("dict_url"), str):
        return node["dict_url"]
    for v in table.values():
        if isinstance(v, dict):
            got = find_dict_url(v, key)
            if got is not None:
                return got
    return None


def check_model_set_alignment() -> list[str]:
    """本文件的文件名清单必须与 Rust 侧 `ModelSet` 逐字对齐。

    为什么要有：两边各写各的文件名（一个是 Python 字面量，一个是 Rust const），
    改模型时漏一边 ⇒ 产品去读一个不存在的文件，而**编译与测试都照绿**。
    这个检查不需要权重在场，所以 `--check` 永远能跑它。
    """
    problems: list[str] = []
    if not MODEL_SET_SRC.is_file():
        return [f"找不到 {MODEL_SET_SRC}"]

    src = MODEL_SET_SRC.read_text(encoding="utf-8")
    # 只认生产那一套 V6_SMALL，别把注释里提到的 v4 也算进来。
    m = re.search(
        r"pub const V6_SMALL: ModelSet = ModelSet \{(.*?)\n\};", src, re.S
    )
    if not m:
        return [f"{MODEL_SET_SRC.name} 里找不到 `pub const V6_SMALL: ModelSet`"]

    body = m.group(1)

    def field(name: str) -> str | None:
        mm = re.search(rf'{name}:\s*"([^"]+)"', body)
        return mm.group(1) if mm else None

    expected = {
        "det": "ppocrv6-small/PP-OCRv6_det_small.onnx",
        "rec": "ppocrv6-small/PP-OCRv6_rec_small.onnx",
        "keys": "ppocrv6-small/ppocrv6_dict.txt",
    }
    for key, want in expected.items():
        got = field(key)
        if got is None:
            problems.append(f"ModelSet.{key} 在 V6_SMALL 里读不出来")
        elif got != want:
            problems.append(f"ModelSet.{key} = {got}，本脚本按 {want} 取")

    # 反向：这两个文件清单本身也要和 ModelSet 的 basename 对得上，
    # 否则上面那三条「期望值」就只是抄了本脚本自己。
    got_names = {n for _, n in FILES} | {DICT_FILE}
    for key, want in expected.items():
        base = Path(want).name
        if base not in got_names:
            problems.append(f"本脚本的清单里没有 {base}（ModelSet.{key} 要用它）")

    return problems



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

    # 这一条**不需要权重在场、也不需要网络**，所以第一个跑：
    # 它挡的是「两边文件名各写各的」这种编译与测试都照绿的错。
    problems = check_model_set_alignment()
    if problems:
        print("文件名与 Rust 侧 ModelSet 对不齐：", file=sys.stderr)
        for p in problems:
            print(f"  · {p}", file=sys.stderr)
        print("改一边必须同时改另一边（ModelSet 在 crates/capture/src/ocr_pp_pipeline.rs）。",
              file=sys.stderr)
        return 1

    local_names = [name for _, name in FILES] + [DICT_FILE]

    if args.check:
        missing = [n for n in local_names if not (DEST / n).is_file()]
        if missing:
            print(f"缺：{missing}（在 {DEST}）", file=sys.stderr)
            return 1
        # 词典额外核 SHA256：它在磁盘上但内容被换过时，「齐全」检查看不出来。
        dict_path = DEST / DICT_FILE
        got = sha256_of(dict_path)
        if got != DICT_SHA256:
            print(
                f"{DICT_FILE} 的 SHA256 不符：\n  期望 {DICT_SHA256}\n  实得 {got}",
                file=sys.stderr,
            )
            return 1
        print(f"权重齐全：{DEST}")
        print(f"ModelSet 文件名对齐 OK；{DICT_FILE} 校验通过 {got[:16]}…")
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

    # 词典随 rec 一起取。**它不在我们用的 onnxruntime 那一支里**（见模块 docstring），
    # 所以 URL 从 rec 那个键的别的 backend 分支拿，校验值本地钉死。
    dict_url = find_dict_url(table, DICT_KEY)
    if dict_url is None:
        print(
            f"模型表里 {DICT_KEY} 下面没有 dict_url。上游改了表结构——"
            "宁可停下，也不要装一个来路不明的词典。",
            file=sys.stderr,
        )
        return 2
    dict_path = DEST / DICT_FILE
    if dict_path.is_file() and sha256_of(dict_path) == DICT_SHA256:
        print(f"已存在且校验通过  {DICT_FILE}")
    else:
        print(f"下载 {DICT_FILE} …")
        with urllib.request.urlopen(dict_url) as r, dict_path.open("wb") as f:
            shutil.copyfileobj(r, f)
        got = sha256_of(dict_path)
        if got != DICT_SHA256:
            dict_path.unlink(missing_ok=True)
            print(
                f"SHA256 不符，已删除：{DICT_FILE}\n"
                f"  期望 {DICT_SHA256}\n"
                f"  实得 {got}\n"
                "  上游可能合法更新了词典 —— 请复核后改本文件里的 "
                "DICT_SHA256 / DICT_LINES。",
                file=sys.stderr,
            )
            return 1
        print(f"  校验通过 {got[:16]}…  {dict_path.stat().st_size / 1e3:.1f} KB")

    lines = len(dict_path.read_text(encoding="utf-8").splitlines())
    if lines != DICT_LINES:
        print(
            f"⚠ 词典是 {lines} 行，本项目按 {DICT_LINES} 行核对"
            "（+blank+space = 18710 类）。不匹配的话 rec 会静默返回空串。",
            file=sys.stderr,
        )
        return 1
    print(f"词典行数 {lines} 与 ModelSet 的 18710 类对得上")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
