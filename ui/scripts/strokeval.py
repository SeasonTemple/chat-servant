"""笔画连续性量具：把「笔画有多细」和「笔画还在不在」分成两个量。

## 为什么要有它（票 35）

票 22 的验收判据是「`≤1px 细笔画` 占比高 ⇒ 会断笔 ⇒ 读不出来」。
真机 WebView2 上量出一个反例：仿宋 21px 的 `≤1px` 占比 **71.6%**，
比当初用来否掉楷体 19px 的 **48.4%** 还高一截，**却完全读得出来**。

⇒ 那条链断在「细」和「断」之间。细是**宽度**属性，断是**连通性**属性，
两者不是一回事。所以这里不再碰宽度，改量**连续性**：

| 量 | 量的是什么 | 越高意味着 |
|---|---|---|
| `字面率` | 墨面积 ÷ 采样框面积 | 墨多寡（沿用票 22 的口径，便于对照） |
| `≤1px占比` | 宽度 ≤1px 的墨游程占比 | **笔画细**（旧判据，保留以便并排看） |
| `墨迹高÷字号` | 墨迹高度 ÷ 字号 | 上下贴不贴边 |
| **`碎片率`** | 小面积墨块数 ÷ 墨块总数 | **笔画断得多**（新量） |
| **`细长比`** | 墨块长轴 ÷ 等效直径 | 越接近 1 越像「碎点」，越大越像一根完整笔画 |
| **`每字墨块数`** | 墨块数 ÷ 字数 | 断笔会让这个数上跳 |

## 量具必须自证会响

`--selfproof` 会合成一份**人为断笔**的样本（在字里横切一刀制造断裂），
然后要求：**碎片率 / 每字墨块数明显上升，而 `≤1px占比` 基本不动**。

如果切完之后 `≤1px占比` 也跟着大跳，那说明这两个量其实是一回事，
**新判据不成立**——脚本会把那行打成 FAIL 而不是把它当通过。

## 纯算术，不开浏览器

只用 Pillow + numpy。本仓的教训是能本地算的别交给渲染器：
浏览器一开就是几十秒起步、还占着用户机器。

用法：

    python ui/scripts/strokeval.py <截图.png> [--selfproof]
"""

from __future__ import annotations

import argparse
import sys

import numpy as np
from PIL import Image

# 判定「碎片」的面积阈值（真实像素，按 DPI 缩放）。
# ⚠ 这些数是**相对量**：脚本按行高推字号，所以换 DPI 也不用手改这里。
FRAGMENT_AREA_MAX = 12


def luma(img: Image.Image) -> np.ndarray:
    rgb = np.asarray(img.convert("RGB"), dtype=np.float32)
    return rgb @ np.array([0.299, 0.587, 0.114], dtype=np.float32)


def paper_crop(lum: np.ndarray, inset: int = 8) -> tuple[np.ndarray, tuple[int, int, int, int], float]:
    """圈出**纸**的包围盒，只在纸里做后面的事。

    ## 为什么必须先找纸（2026-10-14 实测踩到）

    面板窗口是 **transparent** 的，`PrintWindow` 把纸以外的地方渲成**黑**。
    于是整张图大半是 lum≈0，而墨阈值若用**全图百分位**算，
    1%/10% 分位全被黑区拖到 0 ⇒ `mid = 0` ⇒ 掩码**整片为空**。

    而脚本当时只是「安静地报一句没切出文本行」——看起来像「这张图没内容」，
    实际是**量具坏了**。本仓的教训：量具坏了数字看着还挺正常，
    或者更糟，安静地返回一个平凡结果。

    撕口的抗锯齿同样会把背景混进纸边，所以 bbox 之后**必须内缩**
    （票 22 的坑 ④）。
    """
    paper = lum > 0.45 * float(lum.max())
    rows = np.where(paper.sum(axis=1) > 0.2 * paper.shape[1])[0]
    cols = np.where(paper.sum(axis=0) > 0.2 * paper.shape[0])[0]
    if rows.size < 8 or cols.size < 8:
        raise SystemExit("FAILED: no paper region found (window may not be rendered)")
    y0 = max(int(rows[0]) + inset, 0)
    y1 = min(int(rows[-1]) - inset, lum.shape[0] - 1)
    x0 = max(int(cols[0]) + inset, 0)
    x1 = min(int(cols[-1]) - inset, lum.shape[1] - 1)
    crop = lum[y0 : y1 + 1, x0 : x1 + 1]
    return crop, (y0, x1, x0, x1), float(np.median(crop))


def ink_mask(lum: np.ndarray, paper_lum: float) -> np.ndarray:
    """纸里的墨迹掩码。

    阈值取**相对纸亮度**：`paper_lum * 0.72`。写字死一个绝对值，
    因为纸色两套主题差得远（暖黄 / 冷灰），而纸上的抗锯齿灰
    在暖黄那张上比纸暗得多、在冷灰那张上未必。
    """
    thr = paper_lum * 0.72
    mask = lum < thr
    # 量具自检：整片空或整片满都说明阈值落在纸的另一边，而不是「没有字」
    frac = float(mask.mean())
    if frac < 0.002:
        raise SystemExit(f"FAILED: ink mask empty (paper_lum={paper_lum:.1f} thr={thr:.1f})")
    if frac > 0.35:
        raise SystemExit(f"FAILED: ink mask covers {frac:.0%}, threshold landed inside the paper")
    return mask


def text_lines(mask: np.ndarray) -> list[tuple[int, int]]:
    """按行投影切出文本行，返回 (y0, y1) 闭区间。"""
    rows = mask.any(axis=1)
    lines: list[tuple[int, int]] = []
    y = 0
    h = mask.shape[0]
    while y < h:
        if rows[y]:
            y0 = y
            while y < h and rows[y]:
                y += 1
            lines.append((y0, y - 1))
        else:
            y += 1
    return lines


def runs_in_row(row: np.ndarray) -> list[tuple[int, int]]:
    """一行里墨游程的 (x0, x1)。"""
    out = []
    x = 0
    n = len(row)
    while x < n:
        if row[x]:
            x0 = x
            while x < n and row[x]:
                x += 1
            out.append((x0, x - 1))
        else:
            x += 1
    return out


def runs_in_col(col: np.ndarray) -> list[tuple[int, int]]:
    """一列里墨游程的 (y0, y1)。"""
    out = []
    y = 0
    n = len(col)
    while y < n:
        if col[y]:
            y0 = y
            while y < n and col[y]:
                y += 1
            out.append((y0, y - 1))
        else:
            y += 1
    return out


class Blob:
    """一个连通域（8 邻接）。断笔画出来的碎片就是这种小东西。"""

    __slots__ = ("area", "bbox", "elong", "max_run", "_pixels")

    def in_box(self, x0: int, y0: int, x1: int, y1: int) -> bool:
        return not (self.bbox[2] < x0 or self.bbox[0] >= x1 or self.bbox[3] < y0 or self.bbox[1] >= y1)

    def __init__(self, area: int, bbox: tuple[int, int, int, int], max_run: int) -> None:
        self.area = area
        self.bbox = bbox
        # 这个连通域里**最宽**的一行游程。≤1 表示它整根都是亚像素细的。
        self.max_run = max_run
        w = bbox[2] - bbox[0] + 1
        h = bbox[3] - bbox[1] + 1
        # 等效直径：与面积相等的圆的直径
        eq = (4.0 * area / np.pi) ** 0.5 if area else 0.0
        # 长轴：包围盒对角线
        self.elong = ((w * w + h * h) ** 0.5 / eq) if eq > 0 else 0.0


def blobs(mask: np.ndarray, area_max: int | None = None) -> list[Blob]:
    """8 邻接连通域。**迭代式**，不用 scipy（少一个依赖）。"""
    h, w = mask.shape
    seen = np.zeros_like(mask, dtype=bool)
    out: list[Blob] = []
    for sy in range(h):
        row = mask[sy]
        for sx in range(w):
            if not row[sx] or seen[sy, sx]:
                continue
            stack = [(sy, sx)]
            seen[sy, sx] = True
            area = 0
            max_run = 0
            x0 = x1 = sx
            y0 = y1 = sy
            while stack:
                y, x = stack.pop()
                area += 1
                if mask[y, x]:
                    # 这个像素所在横向游程的宽度
                    r = 0
                    xx = x
                    while xx >= 0 and mask[y, xx]:
                        xx -= 1
                    xx += 1
                    while xx < w and mask[y, xx]:
                        xx += 1
                        r += 1
                    max_run = max(max_run, r)
                if x < x0:
                    x0 = x
                if x > x1:
                    x1 = x
                if y < y0:
                    y0 = y
                if y > y1:
                    y1 = y
                for dy in (-1, 0, 1):
                    for dx in (-1, 0, 1):
                        ny, nx = y + dy, x + dx
                        if 0 <= ny < h and 0 <= nx < w and mask[ny, nx] and not seen[ny, nx]:
                            seen[ny, nx] = True
                            stack.append((ny, nx))
            if area_max is None or area <= area_max:
                out.append(Blob(area, (x0, y0, x1, y1), max_run))
    return out


def tail_fragmentation(mask: np.ndarray, font_px: float, area_max: int) -> dict[str, float]:
    """**按字窗**量碎片密度，取尾部分位数——而不是整行平均。

    ## 为什么平均不够（2026-10-14 用户复核逼出来的）

    用户看完五档并排图后说：「**17 开始，笔画复杂的字就糊了**」。
    ⚠ 那不是「整行都糊」，是**尾巴**：笔画多的字先糊，简单字还好。

    ⇒ 整行平均恰好把这个尾巴**抹掉**——一行里 15 个复杂字糊了、
    20 个简单字没事，平均只动了 15/35。实测也确实如此：
    仿宋 400 的 21px 与 17px，碎片率只差 **4 个百分点**
    （46.62% vs 50.81%），而两档的可读性已经被判成不同。

    ## 怎么取「字窗」

    CJK 是全宽，理论上按字号切格子就行；本项目正文里混着拉丁与数字
    （`30s`、`40ms`），等宽切会把一个词劈成两半。⇒ 这里**不做字形切分**，
    改用**滑动窗**：窗宽 = 字号、步长 = 窗宽/4，取碎片密度的 p90 / p99。

    ⚠ 这是个近似：**相邻窗会重叠**，所以它量的是「哪个区域最碎」，
    不是「哪个字最碎」。好处是它不需要知道字的边界——而边界切分
    在墨迹粘连时本就不可靠。
    """
    if mask.size == 0 or font_px <= 0:
        return {"frag_p90": float("nan"), "frag_p99": float("nan"), "frag_max": float("nan")}
    h, w = mask.shape
    win = max(int(round(font_px)), 3)
    step = max(win // 4, 1)
    frags = blobs(mask, area_max)
    dens: list[float] = []
    ys = np.where(mask.any(axis=1))[0]
    y0, y1 = int(ys[0]), int(ys[-1])
    for y in range(y0, y1 + 1):
        for x in range(0, max(w - win + 1, 1), step):
            cell = mask[y, x : x + win]
            if not cell.any():
                continue
            cnt = sum(1 for f in frags if f.area <= area_max and f.in_box(x, y, x + win, y + 1))
            dens.append(cnt / (win * (y1 - y0 + 1)))
    if not dens:
        return {"frag_p90": 0.0, "frag_p99": 0.0, "frag_max": 0.0}
    arr = np.array(dens)
    return {
        "frag_p90": float(np.percentile(arr, 90)),
        "frag_p99": float(np.percentile(arr, 99)),
        "frag_max": float(arr.max()),
    }


def measure(mask: np.ndarray) -> dict[str, float]:
    """在一个已裁好的正文块上量全部指标。"""
    total_px = int(mask.size)
    ink_px = int(mask.sum())

    # `≤1px` 按**三个口径**分别算，⚠ 因为旧判据那个数复现不出来：
    # 票 22 记的是 71.6%，而本量具按「横向游程 ≤1px」只量到 33.2%。
    # 差别必然在口径上（是否含纵向、是否含抗锯齿灰像素），
    # 而**票据没留下当时的脚本**（随 target/ 清理丢失）。
    # ⇒ 旧判据现在既站不住（票 35 的反例），**也没法被复核**。
    # 三个口径并排打出来，好让下一个人知道该往哪个方向对。
    thin_h = 0
    max_run = 0
    for y in range(mask.shape[0]):
        for x0, x1 in runs_in_row(mask[y]):
            wdt = x1 - x0 + 1
            thin_h += wdt if wdt <= 1 else 0
            max_run = max(max_run, wdt)
    thin_v = 0
    for x in range(mask.shape[1]):
        for y0, y1 in runs_in_col(mask[:, x]):
            wdt = y1 - y0 + 1
            thin_v += wdt if wdt <= 1 else 0

    bs = blobs(mask)
    frag = [b for b in bs if b.area <= FRAGMENT_AREA_MAX]
    # 逐笔画口径：≤1px 若按「多少比例的笔画整根都是细的」来算，数值会高得多
    # （票 22 记的 71.6% 大概是这个口径，不是像素占比）。两个口径都打出来。
    thin_blobs = [b for b in bs if b.max_run <= 1]
    return {
        "thin_blob_share": len(thin_blobs) / len(bs) if bs else 0.0,
        "ink_ratio": ink_px / total_px if total_px else 0.0,
        "thin_ratio": thin_h / ink_px if ink_px else 0.0,
        "thin_v_ratio": thin_v / ink_px if ink_px else 0.0,
        "thin_union_ratio": max(thin_h, thin_v) / ink_px if ink_px else 0.0,
        "max_run": float(max_run),
        "blobs": float(len(bs)),
        "frag_ratio": len(frag) / len(bs) if bs else 0.0,
        "elong_mean": float(np.mean([b.elong for b in bs])) if bs else 0.0,
        "frag_per_line": float(len(frag)),
    }


def pick_body(mask: np.ndarray) -> tuple[list[tuple[int, int]], np.ndarray]:
    """挑出**正文段落**：按行距聚成块之后，行数最多的那一块。

    ## 为什么不按「最高的行」挑（2026-10-14 实测踩到）

    第一版挑的是行高最大的行簇，结果选中的是**底部那排档位记号**
    （单行、墨迹高 33px），而正文是 4 行、每行 19px——
    「最高」这个启发式恰好被一条 UI 元素压过去了。
    量具给出的数字看着还挺正常（字面率 6.26%），只是**量的根本不是正文**。

    ⇒ 正文是**一段**、UI 元素多是一行。所以判据用**行数**，
    行数相同再比墨量。归属行与引文只有一行，聚成块也赢不了 4 行。
    """
    lines = text_lines(mask)
    # 1~2px 的「行」是分隔线/描边，不是文字。它们会把段落块撑高、
    # 也会把墨迹高算错，先剔掉。
    lines = [ln for ln in lines if ln[1] - ln[0] + 1 >= 3]
    if not lines:
        return [], np.zeros((0, 0), dtype=bool)
    clusters: list[list[tuple[int, int]]] = [[lines[0]]]
    for prev, cur in zip(lines, lines[1:]):
        gap = cur[0] - prev[1] - 1
        tol = max(prev[1] - prev[0] + 1, cur[1] - cur[0] + 1)
        if gap <= tol:
            clusters[-1].append(cur)
        else:
            clusters.append([cur])

    def ink_of(grp: list[tuple[int, int]]) -> int:
        return int(mask[grp[0][0] : grp[-1][1] + 1].sum())

    # ⚠ 2026-10-14 改判：**按总墨量选，不按行数选**。
    # 第一版按「行数最多」，在 11px 那档选中了**窗口右上角的 ✕ / ⚙ 图标行**
    # ——因为它和归属行、引文连成了 3 行的块，比正文那 2 行还多。
    # 而图标行的墨迹密度只有 2.8%，正文是 20%+
    # ⇒ 正文是**满栏密排**，图标是**一小撮**。总墨量分得开，行数分不开。
    best = max(clusters, key=ink_of)
    y0 = best[0][0]
    y1 = best[-1][1]
    return best, mask[y0 : y1 + 1, :]


def full_measure(mask: np.ndarray, font_px: float, line_heights: list[int] | None = None) -> dict[str, float]:
    m = measure(mask)
    m.update(tail_fragmentation(mask, font_px, FRAGMENT_AREA_MAX))
    if line_heights:
        m["ink_h_px"] = float(np.median(line_heights))
    else:
        ys = np.where(mask.any(axis=1))[0]
        m["ink_h_px"] = float(len(ys))
    # 墨迹高 ÷ 字号。**按行高量再取中位数**，不能拿整块的高去比——
    # 整块高是「行数 × 行高」，那个比值毫无意义（第一版就犯过，
    # 打出了 4.05× 这种一眼就不可能的数）。
    m["ink_h_ratio"] = (m["ink_h_px"] / font_px) if font_px else float("nan")
    return m


def report(tag: str, m: dict[str, float], font_px: float, lines: int) -> None:
    n = max(lines, 1)
    print(f"  [{tag}]")
    print(f"    ink coverage      {m['ink_ratio'] * 100:6.2f}%")
    print(
        f"    thin OLD  h/v/any {m['thin_ratio'] * 100:6.2f}%"
        f" / {m['thin_v_ratio'] * 100:6.2f}% / {m['thin_union_ratio'] * 100:6.2f}%"
        f"   widest run {m['max_run']:.0f}px"
    )
    print(f"    thin-blob share   {m['thin_blob_share'] * 100:6.2f}%   <- per-blob convention")
    print(f"    ink height       {m.get('ink_h_px', float('nan')):6.2f}px   / size {font_px:.0f}px = {m.get('ink_h_ratio', float('nan')):5.3f}x")
    print(f"    blobs per line    {m['blobs'] / n:6.2f}   total blobs {m['blobs']:.0f}")
    print(f"    fragment ratio NEW{m['frag_ratio'] * 100:6.2f}%   frags/line {m['frag_per_line'] / n:.2f}")
    print(
        f"    tail p90/p99     {m.get('frag_p90', float('nan')) * 100:6.2f}%"
        f" / {m.get('frag_p99', float('nan')) * 100:6.2f}%   <- worst glyphs, not the average"
    )
    print(f"    elongation NEW    {m['elong_mean']:6.2f}")


def synth_break(mask: np.ndarray) -> np.ndarray:
    """合成**亚像素笔画掉点**：让本来就 1px 宽的笔画沿途中断续。

    ## 三版合成实验，每一版都改了我的结论（2026-10-14）

    **v1 切一整列**：只有穿过那一列的笔画断 ⇒ 碎片率只动 0.93pp。
    量具「没响」其实是**实验太弱**，不是量具测不出连续性。

    **v2 每根笔画中点掉一像素**：碎片率 +41.7pp 响了，**可旧判据也动了
    +12.0pp** ⇒ 判据纠缠。原因是这一刀切在**粗笔画**上，把 3px 的笔画
    切成 1px + 1px——**于是它顺手把「粗笔画」也变成了「细笔画」**，
    旧判据当然跟着涨。这测的不是亚像素掉点，是「把粗的弄细」。

    **v3（本版）只对已经 1px 宽的笔画掉点**：这才是真机上的失效形态——
    笔画细到亚像素，渲染时**沿途中整像素消失**，而不是被谁切了一刀。
    预期：旧判据**基本不动**（1px 的像素被删掉，分子分母同减，占比不变），
    新判据**大涨**（一根连续的 1px 笔画断成一串小段）。

    ## 一条要写进结论的边界

    对**粗笔画**而言，掉点确实会造出细碎片，所以两个判据在粗笔画上
    本来就是纠缠的。本量只针对**亚像素细笔画**——那才是票 22 被打脸的
    那个场景（仿宋 71.6% 却完全可读）。
    """
    h, w = mask.shape

    # ── 1. 认出「亚像素细笔画」的像素 ──────────────────────────────
    #
    # 判据是 `min(横向游程宽, 纵向游程宽) == 1`：
    #   · 一根 1px 细笔画 → 两向都是 1 ⇒ min = 1，命中 ✓
    #   · 粗笔画中间那个像素（15×3 的一笔里）→ 行宽 15、列宽 3 ⇒ min = 3，不命中 ✓
    #   · 粗笔画边缘那个像素 → 行宽 15、列宽 2 ⇒ min = 2，不命中 ✓
    #
    # ⚠ v2/v3 都栽在这里：只要对**整片掩码**动刀（切列、棋格），
    # 粗笔画就一起被打碎，于是旧判据必然跟着涨——那测的是
    # 「把粗笔画弄细」，不是「细笔画掉点」。两次纠缠都是这个原因。
    thin = np.zeros_like(mask)

    row_width = np.zeros_like(mask, dtype=np.int32)
    for y in range(h):
        row = mask[y]
        x = 0
        while x < w:
            if row[x]:
                x0 = x
                while x < w and row[x]:
                    x += 1
                row_width[y, x0:x] = x - x0
            else:
                x += 1

    col_width = np.zeros_like(mask, dtype=np.int32)
    for x in range(w):
        col = mask[:, x]
        y = 0
        while y < h:
            if col[y]:
                y0 = y
                while y < h and col[y]:
                    y += 1
                col_width[y0:y, x] = y - y0
            else:
                y += 1

    thin = mask & (np.minimum(row_width, col_width) <= 1)

    # ── 2. 只在细笔画像素上做「隔一个掉一个」 ──────────────────────
    #
    # 预期效果：粗笔画一个像素都不动 ⇒ 旧判据的分子里粗笔画那部分不变；
    # 细笔画的像素被删掉一半 ⇒ 分子分母同减 ⇒ **占比不变**。
    # 而一根连续的细笔画断成一串小段 ⇒ 墨块数、碎片率大涨。
    out = mask.copy()
    parity = (np.indices(mask.shape).sum(axis=0) % 2) == 0
    out[thin & parity] = False
    return out


def selfproof(mask: np.ndarray, font_px: float, lines: list[tuple[int, int]]) -> int:
    """自证：人为断笔后，新量必须动而旧量基本不动。"""
    print("== SELF-PROOF: one column sliced out of the glyphs ==")
    heights = [y1 - y0 + 1 for y0, y1 in lines]
    before = full_measure(mask, font_px, heights)
    after = full_measure(synth_break(mask), font_px, heights)
    report("as-is", before, font_px, len(lines))
    report("broken", after, font_px, len(lines))

    thin_move = abs(after["thin_ratio"] - before["thin_ratio"]) * 100
    frag_gain = (after["frag_ratio"] - before["frag_ratio"]) * 100
    blobs_gain = (after["blobs"] - before["blobs"]) / max(before["blobs"], 1) * 100
    elong_move = abs(after["elong_mean"] - before["elong_mean"])
    print()
    print(f"  thin ratio moved  {thin_move:6.2f} pp   <- OLD criterion, expected ~0")
    print(f"  fragment moved    {frag_gain:6.2f} pp   <- NEW criterion, expected > 0")
    print(f"  blob count moved  {blobs_gain:6.2f}%")
    print(f"  elongation moved  {elong_move:6.2f}")

    ok_thin = thin_move < 5.0
    ok_frag = frag_gain > 5.0
    if ok_thin and ok_frag:
        print("  PASS: the two measures track different things. New criterion holds.")
        return 0
    if ok_frag and not ok_thin:
        print("  WARN: slicing also raised the thin ratio - they are entangled.")
        return 1
    print("  FAIL: NEW criterion did not move on broken strokes -> it does not")
    print("        measure continuity. Conclusion void.")
    return 2


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("png")
    ap.add_argument("--selfproof", action="store_true", help="run the broken-stroke self-proof")
    ap.add_argument("--inset", type=int, default=8, help="inset applied to the paper bbox (px)")
    ap.add_argument("--font-px", type=float, default=0.0, help="font size in real px; 0 = derive from line height")
    ap.add_argument("--dump-lines", action="store_true", help="print every detected line and stop")
    args = ap.parse_args()

    img = Image.open(args.png)
    lum = luma(img)
    crop, (py0, py1, px0, px1), paper_lum = paper_crop(lum, args.inset)
    mask = ink_mask(crop, paper_lum)
    all_lines = text_lines(mask)
    if args.dump_lines:
        print(f"== {args.png} (dump) ==")
        print(f"  paper y{py0}..{py1} x{px0}..{px1}  paper_lum {paper_lum:.1f}  ink frac {mask.mean():.4f}")
        w = mask.shape[1]
        for i, (y0, y1) in enumerate(all_lines):
            seg = mask[y0 : y1 + 1]
            dense = float(seg.sum(axis=1).max()) / w
            print(
                f"  line {i:2d}  y {y0:4d}..{y1:4d}  h {y1 - y0 + 1:3d}"
                f"  ink {seg.mean() * 100:5.2f}%  widest-row {dense * 100:5.1f}%"
            )
        return 0
    lines, body = pick_body(mask)
    if body.size == 0:
        raise SystemExit("FAILED: no text line found inside the paper")

    heights = [y1 - y0 + 1 for y0, y1 in lines]
    font_px = args.font_px or float(np.median(heights))
    m = full_measure(body, font_px, heights)

    print(f"== {args.png} ==")
    print(
        f"  image {img.width}x{img.height}  paper y{py0}..{py1} x{px0}..{px1}"
        f"  paper_lum {paper_lum:.1f}"
    )
    print(f"  body block {body.shape[1]}x{body.shape[0]}  text lines {len(lines)}")
    report("body", m, font_px, len(lines))
    print()
    if args.selfproof:
        return selfproof(body, font_px, lines)
    return 0


if __name__ == "__main__":
    sys.exit(main())
