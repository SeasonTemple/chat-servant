#!/usr/bin/env python3
"""量一段中文小字的「断笔画」风险：字面率 / ≤1px 细笔画 / 墨迹高÷字号。

为什么需要它
------------
票 24 要回答的是「面板上最小的字到底能不能读」。v1 主张下限 12px，
理由是**字体的物理限制**（小字在纸纹上会断笔）——而那个理由从来没被检验过。

断笔的后果**不是「看着挤」，是「读不出来」**，量级不同，所以必须量。

⚠ **为什么这个脚本住在 `ui/scripts/` 而不是 `target/`**
   票 24 的量具原来放在 `target/typo/analyse-type.py`，2026-10-13 那次
   `target/` 清理把它一起删了，于是「用 §4.2 那套」这条指示当场落空。
   **量具不能放在会被清理的目录里。**

⚠ **首轮数据为什么作废**
   一个 sheet 里混排多种行（汉字行 + 等宽行），行切分按「比纸暗」判定，
   等宽行的墨也进了掩码，切出来的行于是横跨两种行，量出来的东西
   不属于任何一种字号。现在工作台一次只出一档（`/studio?only=small&small=N`），
   从源头上避免。

量的三个东西
------------
1. **字面率** = 墨像素 ÷ 该行的外接框面积。太高 = 挤成一团，太低 = 发虚。
2. **≤1px 细笔画占比** = 笔画宽度 ≤ 1 CSS px 的墨像素 ÷ 全部墨像素。
   **这是断笔的直接指标**：抗锯齿之后 1px 的笔画在纸纹上会断。
3. **墨迹高 ÷ 字号** = 墨的外接框高 ÷（字号 × 渲染倍率）。
   明显小于 1 说明字被行高压扁或栅格化吃掉了行。

用法
----
    python ui/scripts/analysetype.py target/type/small-11.png --size 11 --scale 2

⚠ `--scale` 必须与出图时的 `--force-device-scale-factor` 一致，
   否则「1px 笔画」这条判据会整体错位（见下面 `scale` 参数说明）。
"""

import argparse
import sys
from pathlib import Path

from PIL import Image

# 「墨」的亮度上限。⚠ 用的是 PIL 的 `L`（ITU-R 601 luma），**不是** WCAG
# 相对亮度——为的是能走字节级比较，不必逐像素算幂函数。
# 两者在本量具上**分类结果相同**，因为阈值离两端都很远：
#   铅笔灰 rgb(76 90 99)   → luma 87
#   纸底   rgb(243 230 192)→ luma 230
# ⇒ 阈值 150 只在**纸内**用，见 `paper_region`。
INK_MAX_LUMA = 150

# 「纸」的判据：**暖**且亮。R 比 B 高出一截是奶白纸与工作台冷灰蓝底
# 最稳的区别（底色 R≈B≈G，几乎没有色相偏移）。
#
# ⚠⚠ **为什么必须先定位纸再取墨**（初版就死在这儿）
# 工作台底 `rgb(74 85 104)` 的 luma 是 **110**，**低于 150**。
# 初版直接对整图取「比 150 暗的像素」当墨 ⇒ **整页都是墨**：
# 行切分切出一条贯穿全图的大带，逐像素算笔画宽度跑了 4.8 M 像素也出不来，
# 而且**报出来的那张表看起来完全正常**——只是慢。
# ⇒ 先框出纸，只在纸里取墨。两道判据各自都有自检。
PAPER_MIN_LUMA = 170
PAPER_MIN_WARMTH = 20      # R - B 的最小值

# 同一行内允许的空隙（CSS px）。超过它就切成两行。
ROW_GAP_PX = 3.0


def _warm_mask(im, min_luma, min_warmth):
    from PIL import ImageChops

    rgb = im.convert("RGB")
    r, g, b = rgb.split()
    warm = ImageChops.subtract(r, b).point(lambda v: 255 if v >= min_warmth else 0, mode="L")
    bright = g.point(lambda v: 255 if v >= min_luma else 0, mode="L")
    return ImageChops.multiply(warm, bright)


def _components(mask):
    """按行游程 + 并查集做连通块，返回每个块的 (x0, y0, x1, y1)。"""
    w, h = mask.size
    data = mask.tobytes()
    parent = {}

    def find(a):
        while parent[a] != a:
            parent[a] = parent[parent[a]]
            a = parent[a]
        return a

    def union(a, b):
        ra, rb = find(a), find(b)
        if ra != rb:
            parent[rb] = ra

    prev = []
    for y in range(h):
        row = data[y * w:(y + 1) * w]
        runs = []
        x = 0
        while x < w:
            if row[x]:
                x2 = row.find(0, x)
                if x2 == -1:
                    x2 = w
                runs.append((x, x2))
                x = x2
            else:
                x += 1
        cur = []
        for (a, b) in runs:
            parent[(y, a)] = (y, a)
            cur.append((a, b))
            for (pa, pb) in prev:
                if pa < b and a < pb:
                    union((y - 1, pa), (y, a))
        prev = cur

    boxes = {}
    for key in list(parent):
        root = find(key)
        y, x = key
        if root not in boxes:
            boxes[root] = [x, y, x, y]
        else:
            bx = boxes[root]
            bx[0] = min(bx[0], x)
            bx[2] = max(bx[2], x)
            bx[3] = max(bx[3], y)
    return [tuple(v) for v in boxes.values()]


def _fill_rows(mask):
    """把纸按行**填实**（行内从最左纸像素连到最右纸像素）。

    ⚠ 为什么必须填：文字是**画在纸上**的，它替换掉了那些像素，
    所以「纸掩码 ∧ 墨掩码」会把文字自己滤掉——初版就是这么量出 0 行的。
    而**不能**直接取外接框：框里纸上缘/下缘之外是工作台底色（luma 110，
    比墨阈值 150 还暗），不排除就会把那一圈当成墨，整块并成一行。
    ⇒ 填行：纸的**内部**（含文字处）算纸，纸的**外部**不算。
    """
    w, h = mask.size
    data = mask.tobytes()
    out = bytearray(w * h)
    for y in range(h):
        row = data[y * w:(y + 1) * w]
        try:
            a = row.index(255)
        except ValueError:
            continue
        b = w
        while b > a and row[b - 1] != 255:
            b -= 1
        base = y * w
        out[base + a:base + b] = b"\xff" * (b - a)
    return out


def paper_region(im, min_luma, min_warmth, ink_threshold):
    """框出**要量的那块纸**，返回 (bbox, 填实后的纸掩码字节)。

    ⚠⚠ **为什么不能取「所有暖色像素的并集」**（初版就死在这儿 twice）
    工作台上除了候选纸带，左侧还常驻一块**纸样参照**（380×560 那张大纸）。
    取并集等于把两者框在一起——于是量到的是**空白的参照纸**：
    0 行墨、字面率 49.5%、墨高÷字号 47.6，而这张表**看起来完全正常**。
    ⇒ 必须选**含墨最多的那块纸**：参照纸上一行字都没有。

    连通块用行游程 + 并查集算，块数很少，纯 Python 足够。
    """
    paper = _warm_mask(im, min_luma, min_warmth)
    boxes = _components(paper)
    if not boxes:
        raise SystemExit(
            "ABORT: no paper-coloured region found. The workbench did not render "
            "the paper band (or --only / --small is wrong). Print the image and "
            "look before changing PAPER_MIN_LUMA / PAPER_MIN_WARMTH."
        )
    best_box, best_ink = None, -1
    for box in boxes:
        x0, y0, x1, y1 = box
        # 面积太小的块是抗锯齿的碎屑，不是纸。
        if (x1 - x0) * (y1 - y0) < 4000:
            continue
        n = ink_mask(im.crop(box), ink_threshold).tobytes().count(255)
        if n > best_ink:
            best_ink, best_box = n, box
    if best_box is None or best_ink == 0:
        raise SystemExit(
            "ABORT: no paper region contains any ink. The sample did not render, "
            "or the paper block picked is the empty reference sheet."
        )
    return best_box, _fill_rows(paper.crop(best_box))


def ink_mask(im, threshold):
    """二值墨掩码（L 模式，0/255），整图操作走 PIL 的 C 实现。"""
    return im.convert("L").point(lambda v: 255 if v < threshold else 0, mode="L")


def row_bands(mask, w, h, gap):
    """按「墨行之间有没有空隙」切行，返回每行的 [y0, y1]。"""
    data = mask.tobytes()
    bands = []
    start = None
    blank = 0
    for y in range(h):
        filled = data.count(255, y * w, (y + 1) * w)
        if filled:
            if start is None:
                start = y
            blank = 0
        elif start is not None:
            blank += 1
            if blank > gap:
                bands.append((start, y - blank))
                start = None
    if start is not None:
        bands.append((start, h - 1))
    return bands


def stroke_widths(sub, scale):
    """逐墨像素估笔画宽度 = min(水平连续长度, 垂直连续长度)。

    为什么取 min：汉字的横画水平长、竖画垂直长，各取其一都会把
    交叉处的高估带进来；min 给出的是**笔画的粗**而不是**笔画的长**。

    `sub` 是那一行裁出来的小图（不是整图）——所以这个 Python 循环只跑
    文字那一小块，不是 3000×1600。⚠ 这正是脚本能跑完的原因：
    初版对整张图逐像素算，4.8 M 像素 × 3 张，几分钟都出不来。
    """
    w, h = sub.size
    px = sub.load()
    widths = []
    for y in range(h):
        for x in range(w):
            if px[x, y] != 255:
                continue
            run_h = 1
            xx = x - 1
            while xx >= 0 and px[xx, y] == 255:
                run_h += 1
                xx -= 1
            xx = x + 1
            while xx < w and px[xx, y] == 255:
                run_h += 1
                xx += 1
            run_v = 1
            yy = y - 1
            while yy >= 0 and px[x, yy] == 255:
                run_v += 1
                yy -= 1
            yy = y + 1
            while yy < h and px[x, yy] == 255:
                run_v += 1
                yy += 1
            widths.append(min(run_h, run_v))
    return widths


def _drop_isolated_components(sub, neighbour_px=4):
    """剔掉**孤立的**连通域——纸纹底上的暗折痕。

    ⚠⚠ **默认不启用**（`--drop-isolated` 才开）
    纸纹位图里有暗折痕，它比墨阈值还暗，会把一行的包围盒撑宽
    （实测「示例联系人」那行被撑到 229 CSS px，真值约 60）。
    折痕与 11px 的竖笔同形、密度也同形，按形状和密度都剔不掉，
    **只能按「成簇」剔**：竖笔旁边必有横笔，字形成串；
    折痕左右是空白纸，是孤立的。

    ⚠ 而它会**误伤真字**：纯色底上不需要它，硬开就会把只填入里
    恰好孤立的那一笔丢掉——实测 12px 那一行量出 23 CSS px，
    **比 10.5px 的 31 px 还窄**，而三个字号只该单调变宽。
    ⇒ 纯色底（`--paper solid`）一律**不开**这个过滤。
    """
    w, h = sub.size
    data = sub.tobytes()
    boxes = _components(Image.frombytes("L", (w, h), bytes(data)))
    if not boxes:
        return sub
    keep = []
    for i, (x0, y0, x1, y1) in enumerate(boxes):
        clustered = any(
            j != i and not (b1 < x0 - neighbour_px or x1 + neighbour_px < b0)
            for j, (b0, _c0, b1, _c1) in enumerate(boxes)
        )
        if clustered:
            keep.append((x0, y0, x1, y1))
    if not keep:
        return sub
    out = bytearray(w * h)
    for (x0, y0, x1, y1) in keep:
        for y in range(y0, y1 + 1):
            base = y * w
            out[base + x0:base + x1 + 1] = data[base + x0:base + x1 + 1]
    return Image.frombytes("L", (w, h), bytes(out))


def analyse(path, font_px, scale, threshold, drop_isolated=False):
    im = Image.open(path).convert("RGB")
    box, paper_fill = paper_region(im, PAPER_MIN_LUMA, PAPER_MIN_WARMTH, threshold)
    print(f"{path.name}   image={im.size[0]}x{im.size[1]}  scale={scale} "
          f"font={font_px}px  paper_bbox={box}")
    # ⚠⚠ **墨必须落在纸上**（初版在这里死过两次）。
    # 直接取外接框：框里纸上缘/下缘之外是**工作台底色**，luma 110
    # ——比墨阈值 150 还暗，那一圈会被当成墨，整块并成**一行 96 CSS px**。
    # 直接取「纸掩码 ∧ 墨掩码」：文字替换了纸的像素，于是文字被滤掉 → 0 行。
    # ⇒ 用 `paper_region` 里**按行填实**的纸掩码：纸的内部（含文字处）算纸。
    ink_local = ink_mask(im.crop(box), threshold)
    data_i = ink_local.tobytes()
    data_p = bytes(paper_fill)
    mask = Image.frombytes(
        "L", ink_local.size, bytes(255 if (a and b) else 0 for a, b in zip(data_i, data_p))
    )
    w, h = mask.size
    bands = row_bands(mask, w, h, int(ROW_GAP_PX * scale))

    # ⚠ **剔掉贴着纸上缘的那一行**：那是纸的**撕口暗边**，不是字。
    # 工作台的纸带是 `.band.grow`（上 padding 14px），文字必然离上缘有一段
    # 距离，于是「上边界 = 0」就是撕口边的判据。留着它会把字面率与细笔画
    # 掺进一条与文字无关的暗纹——而表里只是多一行，看不出异常。
    torn = [b for b in bands if b[0] == 0]
    bands = [b for b in bands if b[0] != 0]
    print(f"  paper region analysed: {w}x{h}px   text rows: {len(bands)}  "
          f"(torn-edge rows dropped: {len(torn)})  each render = ONE font size")
    if not bands:
        raise SystemExit(
            "ABORT: no ink found inside the paper region. Either the page did not "
            f"render the sample, or threshold {threshold} is below the paper luma."
        )

    rows = []
    for i, (y0, y1) in enumerate(bands):
        sub = mask.crop((0, y0, w, y1 + 1))
        if drop_isolated:
            sub = _drop_isolated_components(sub)
        bb = sub.getbbox()
        if bb is None:
            continue
        sub = sub.crop(bb)
        sw, sh = sub.size
        ink = sum(1 for v in sub.tobytes() if v == 255)
        widths = stroke_widths(sub, scale)
        if not widths:
            continue
        thin = sum(1 for v in widths if v / scale <= 1.0)
        rows.append({
            "i": i,
            "y": y0 / scale,
            "w": sw / scale,
            "h_css": sh / scale,
            "fill": 100.0 * ink / (sw * sh),
            "thin": 100.0 * thin / len(widths),
            "h_ratio": (sh / scale) / font_px,
        })

    print(f"  {'行':<3}{'y':>7}{'宽css':>8}{'墨高css':>9}{'字面率':>9}"
          f"{'≤1px细笔画':>12}{'墨高÷字号':>11}")
    for r in rows:
        print(f"  {r['i']:<3}{r['y']:>7.1f}{r['w']:>8.1f}{r['h_css']:>9.1f}"
              f"{r['fill']:>8.1f}%{r['thin']:>11.1f}%{r['h_ratio']:>11.2f}")

    # ── self-check 1: 恰好三行 ────────────────────────────────────────
    # 工作台的 D 段固定排三行：归属行两行（名字 + 引文）+ 档位名一行。
    # 撕口那一行已被剔掉。多一行 = 切碎或混进了纸纹；少一行 = 并行或漏切。
    # ⚠ 这条是「量具偏了要能报出来」的那道闸——初版就是 4 行（含撕口）
    # 一路绿灯跑到了结论那一行。
    if len(rows) != 3:
        raise SystemExit(
            f"ABORT: {len(rows)} text rows found, expected exactly 3 "
            "(where line = 2 lines + the pick-name line). More = a row was split or "
            "paper grain leaked in; fewer = two rows merged or one was lost. "
            "Print the image and look before changing ROW_GAP_PX."
        )

    # ── self-check 2: 墨高÷字号必须落在一个说得通的区间 ───────────────
    # 明显 < 0.5 说明行被切碎或取样框只吃到了半行；明显 > 1.2 说明
    # 字号与出图倍率对不上——两者都会让「细笔画占比」变成另一个字号的数。
    for r in rows:
        if not (0.5 <= r["h_ratio"] <= 1.2):
            raise SystemExit(
                f"ABORT: row {r['i']} ink height / font size = {r['h_ratio']:.2f}, "
                "outside 0.5..1.2. Either the row is cut wrong, or --scale / --size "
                "does not match how the image was rendered."
            )
    return rows


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("png")
    ap.add_argument("--size", type=float, required=True, help="该图渲染的 CSS 字号")
    ap.add_argument("--scale", type=float, required=True,
                    help="出图时的 --force-device-scale-factor")
    ap.add_argument("--threshold", type=int, default=INK_MAX_LUMA)
    ap.add_argument("--drop-isolated", action="store_true",
                    help="剔掉孤立连通域（只用于**纸纹底**：折痕会撑宽包围盒）")
    a = ap.parse_args()
    analyse(Path(a.png), a.size, a.scale, a.threshold, a.drop_isolated)


if __name__ == "__main__":
    sys.exit(main())
