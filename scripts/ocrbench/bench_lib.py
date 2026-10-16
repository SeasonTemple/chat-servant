"""OCR 评测的尺子：规范化、编辑距离、ground truth ↔ 识别行的对齐。

**两条路线必须用同一把尺子**，所以这个模块不含任何引擎相关的分支——
引擎只负责交出 `(text, y0, y1)` 三元组，其余一律在这里算。

## 规范化（唯一出处）

`normalize()` 三步，顺序固定：

1. `unicodedata.normalize("NFKC", s)` —— 全角转半角。NFKC 是 Unicode
   标准的兼容分解，比手写「哪些是全角」的映射表可靠得多。
2. 删掉**全部** Unicode 空白（`str.isspace()` 逐字符判定）。
3. `str.lower()` —— 只做大小写折叠，不做 casefold（后者会把 ß 展成 ss）。

**为什么删光所有空白**：WinRT 的行组装会把英文单词拆开后又粘死，
实测 `OK, I'll bring the report v2` 被读成 `OK, bringthereportv2`
（`xtask ocr` 输出里间隙比值全是 0.2~2.0，说明是逐词切开再拼的）。
保留空白的话，这一类差异会以每行数个空格的形式灌进编辑距离，
把「词间空格丢了」和「字认错了」这两件**修法完全不同**的事混成一个数。

⚠ **但这意味着删空白对路线 A 是偏袒的**，所以另有一把
`normalize_strict()`：NFKC + 小写，但把连续空白压成**一个**空格、
再去掉首尾空白。两者一起报，差额就是「空白策略贡献的误差」。
"""

from __future__ import annotations

import re
import unicodedata

# 行带相交至少要覆盖较矮一方的一半，才算「对应上了同一条消息」。
OVERLAP_RATIO = 0.5
# 相似度低于此值就算漏检（任务书指定的阈值口径）。
MISS_SIM = 0.5

_WS_RUN = re.compile(r"\s+")


def normalize(s: str) -> str:
    """主口径：删光空白 + NFKC + 小写。"""
    s = unicodedata.normalize("NFKC", s)
    s = "".join(ch for ch in s if not ch.isspace())
    return s.lower()


def normalize_strict(s: str) -> str:
    """对照口径：保留单个空格，只做 NFKC + 小写 + 压空白 + 去首尾。"""
    s = unicodedata.normalize("NFKC", s)
    s = _WS_RUN.sub(" ", s).strip()
    return s.lower()


def levenshtein(a: str, b: str) -> int:
    """标准编辑距离（插入/删除/替换各 1），滚动数组实现。"""
    if a == b:
        return 0
    if not a:
        return len(b)
    if not b:
        return len(a)
    prev = list(range(len(b) + 1))
    for i, ca in enumerate(a, 1):
        cur = [i]
        for j, cb in enumerate(b, 1):
            cur.append(min(prev[j] + 1, cur[j - 1] + 1, prev[j - 1] + (ca != cb)))
        prev = cur
    return prev[-1]


def similarity(a: str, b: str) -> float:
    """归一化后的相似度。分母取较长者，空串给 0。"""
    if not a and not b:
        return 1.0
    m = max(len(a), len(b))
    if m == 0:
        return 1.0
    return 1.0 - levenshtein(a, b) / m


class Hyp:
    """一条识别行。只保留评测需要的三个字段。"""

    __slots__ = ("text", "y0", "y1")

    def __init__(self, text: str, y0: int, y1: int) -> None:
        self.text = text
        self.y0 = y0
        self.y1 = y1


class Judgement:
    """一条 ground truth 行的判定结果。"""

    __slots__ = ("gt", "hyp", "band", "edit", "sim", "geom_hit")

    def __init__(self, gt: str, band: list, hyp: str | None, edit: int, sim: float,
                 geom_hit: bool) -> None:
        self.gt = gt
        self.band = band
        self.hyp = hyp
        self.edit = edit
        self.sim = sim
        self.geom_hit = geom_hit


def align(gt_lines: list[dict], hyps: list[Hyp], norm=normalize) -> list[Judgement]:
    """把 ground truth 行与识别行配对。

    ## 为什么用**纵向**重叠而不是按顺序配对

    两条路线都会把同一屏里的行按大致阅读顺序吐出来，但顺序未必一致，
    且路线 A 只给出 `y` 与行高、**不给 x**（见 `xtask ocr` 输出格式），
    横向定位根本无从谈起。纵向带足以区分——气泡之间本来就有间隔。
    所以判据是「纵向带相交覆盖较矮一方 ≥ 50%」。

    配对后取**编辑距离最小**的那条候选，而不是第一条：
    行被切碎时会有多个候选框，取最近的那个才反映真实识别质量。
    """
    out: list[Judgement] = []
    for gt in gt_lines:
        y0, y1 = gt["band"][1], gt["band"][3]
        gt_n = norm(gt["text"])
        span = max(1, y1 - y0)

        cands: list[tuple[int, Hyp]] = []
        for h in hyps:
            hs = max(1, h.y1 - h.y0)
            ov = min(y1, h.y1) - max(y0, h.y0)
            if ov <= 0:
                continue
            if ov < OVERLAP_RATIO * min(span, hs):
                continue
            cands.append((levenshtein(gt_n, norm(h.text)), h))

        if not cands:
            # 一个重叠的框都没有 —— 这是**几何漏检**：引擎根本没提这一行。
            out.append(Judgement(gt["text"], gt["band"], None, len(gt_n), 0.0, False))
        else:
            # 有框但字全错，也仍然是「检出了」。两者要分开报，
            # 因为修法不同：一个要调 det 阈值，一个要调 rec / 行组装。
            d, h = min(cands, key=lambda t: t[0])
            hyp_n = norm(h.text)
            sim = 1.0 - d / max(1, max(len(gt_n), len(hyp_n)))
            out.append(Judgement(gt["text"], gt["band"], h.text, d, sim, True))
    return out


def score(judgements: list[Judgement], norm=normalize) -> dict:
    """一组判定的汇总。分母是**标准答案**的归一化字符数。"""
    edits = 0
    denom = 0
    missed = 0
    geom_miss = 0
    for j in judgements:
        g = norm(j.gt)
        if not g:
            # 标准答案归一化后为空（例如整行是空格）——它不该进分母，
            # 留着会让分母虚高、CER 假性变好。
            continue
        edits += j.edit
        denom += len(g)
        if j.sim < MISS_SIM:
            missed += 1
        if not j.geom_hit:
            geom_miss += 1
    n = sum(1 for j in judgements if norm(j.gt))
    return {
        "n_gt_lines": n,
        "gt_chars": denom,
        "edits": edits,
        "cer": edits / denom if denom else None,
        "miss_rate": missed / n if n else None,
        "geom_miss_rate": geom_miss / n if n else None,
    }
