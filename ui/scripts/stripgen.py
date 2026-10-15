"""从面板自己的纸上裁下真撕边，做成候选行的九宫格 —— 票 44。

## 为什么必须是「裁」而不是「画」

第一版（2026-10-16 上午）用 PIL 在纸芯上**画**出一条 1px 深的密度线当撕口。
用户当场否掉：「边缘得是和面板的那种一致啊？你这样和黑框有什么区别？」

**「黑框」不是修辞，是那条线的实际读数。** `ui/scripts/_tearprobe.py` 量了
面板自己的撕边（`target/probe/tearprobe.txt`）：

| | 面板真撕边（实测） | 第一版画出来的 |
|---|---|---|
| 内缩深度 | **12~27 源 px**（6~13.5 CSS px），众数 17~25 | 1 源 px（0.5 CSS px） |
| alpha 剖面 | `0 → 3 → 53~111 → 203~248 → 254`，**两级过渡** | 一刀切到满不透明 |
| 沿边的变化 | 每 2 px 一采，深度在 12~27 之间连续起伏 | 每 3 px 一个凸起，**有周期** |
| 外侧 | 完全透明（纸没到那儿） | 纸直接顶到边 |

⇒ 真撕口的可辨识度**几乎全部来自那 6~13 CSS px 的深度与两级 alpha 过渡**。
压成 1px 之后剩下的就是「一圈均匀的深线」——那正是黑框的构造方式。

⚠ **所以本文件不再合成任何像素。** 纸的原始生成方式已不可考（规范 §2.4），
但**位图可复现**，而真边就烘在这张位图里。裁下来的边与面板的边
**逐像素同源**，一致性不靠调参数维持。

## slice 的值 = 真撕边的深度，不是审美参数

`SLICE = 32`（源 px @2x = 16 CSS px）。依据：

- 实测顶边内缩众数在 17~25 源 px，最深 27（`_tearprobe.py` 的直方图）。
- 取 **32** 是为了**把整条撕边的渐变吃进来**：alpha 从 0 爬到 254 要 4~5 源 px，
  加上深度最深处 27 ⇒ 32 够覆盖最深的凹口且不留半截渐变。
- ⚠ 若 slice 小于最深凹口，九宫格会在边中间截断渐变，接缝处出现一条
  **半透明的硬边**——那比黑框更糟，因为它看起来像折痕。
  ⇒ slice 必须 ≥ max(实测内缩)，本文件 `build()` 里有断言。

## ⚠ slice 与 CSS 的 border-width 必须一起改

九宫格的 `slice` 是源图像素，`border-width` 是 CSS 像素。@2x 下二者差 2 倍，
但**它们共同决定纸条有多厚**：`border-image ... 6 fill / 3px / 0 stretch` 里
那个 `3px` 才是纸条的实际边框厚度，`slice 6` 只决定从图上取哪块。

⇒ 本文件末尾会打印 CSS 那边必须填的值，`PaperPanel.svelte` 的
`.cs-act { border-width }` 与它**逐字绑定**。改一边必须改另一边。

跑：uv run --with pillow python ui/scripts/stripgen.py
"""

from __future__ import annotations

import sys
from pathlib import Path

from PIL import Image

HERE = Path(__file__).resolve().parent.parent  # ui/
SRC = HERE / "static" / "probe-sheet.png"
OUT = HERE / "static" / "strip.png"
REPORT = Path("target/probe/stripgen-report.txt")

# 九宫格每边多少源 px（源图 @2x）。
# ⚠⚠ **这个数被实测钉死**：必须 ≥ 撕边最深处的内缩（实测 27 源 px），
#   否则渐变会被九宫格截断，接缝露出半透明硬边。
#   留 5 源 px 余量 ⇒ 32。**不要为了「薄一点」调小它。**
SLICE = 32

# ⚠ 源图 760x1120，@2x ⇒ 760/32 = 23.75，得四舍五入成 24 格。
#   96 = 24 × 4，刚好四等分，每条边 32 px —— 九宫格要求源图能被 slice 整除。
SIDE = SLICE * 4  # 128 px 源图 @2x = 64 CSS px

# 从面板的四条边上分别取一段**中段**（避开四角）。
# ⚠ 为什么避角：四角内缩最深到 27+，且角部沿两个方向同时撕裂，
#   轮廓比边中段乱得多。中段是最能代表「这条边长什么样」的地方。
SRC_W, SRC_H = 760, 1120

# 面板真撕边的内缩：顶边众数在 17~25 源 px，最深 27。
# 断言用 —— 源位图哪天被换了而这个数对不上，必须当场报错而不是静默出坏图。
EXPECT_MIN_INSET = 10   # 顶边最浅处（x=304 处实测 12，取 10 留余量）
EXPECT_MAX_INSET = 30   # 顶边最深处（实测 27）

L: list[str] = []
p = L.append


def write_report() -> None:
    REPORT.parent.mkdir(parents=True, exist_ok=True)
    REPORT.write_text("\n".join(L), encoding="utf-8")


def fail(msg: str) -> None:
    """失败时**也要落报告**。

    ⚠ 第一版没有这一步，于是自检拦下之后我只看到一行 GBK 乱码 stderr，
    完全不知道是哪条边不合格 —— 而「知道哪一条不合格」正是修它的前提。
    ⇒ 硬拦与可读的失败出口是两件事，缺一不可。
    """
    p("")
    p(f"❌ 中止：{msg}")
    write_report()
    print(f"written: {REPORT}  (FAILED)")
    sys.exit(1)


def first_opaque(px, x: int, thr: int) -> int:
    for y in range(SRC_H):
        if px[x, y][3] >= thr:
            return y
    return -1


def build() -> None:
    if not SRC.exists():
        sys.exit(f"找不到 {SRC}")

    paper = Image.open(SRC).convert("RGBA")
    px = paper.load()
    amax = max(px[x, y][3] for y in range(0, SRC_H, 3) for x in range(0, SRC_W, 3))
    thr = amax - 6

    # ── 1. 先把真撕边的深度量出来，顺带守住源图没被换掉 ──────────
    #
    # ⚠⚠ **只量中段，别整幅扫**（第一版踩到）：
    #   整幅扫含**四个角**，而角上两条边同时撕，内缩会一路吃到几百 px
    #   （实测 895 / 1022）——那些是真值，不是坏数据。于是
    #   `max(insets)` 变成 1022，断言当场炸掉，而「炸掉」的原因是
    #   **量具取错了样本**，不是源位图换了。
    #   ⇒ 角部另有专门处理（`EXPECT_*` 断言的是边中段的分布）。
    insets: list[int] = []
    for x in range(SRC_W // 5, SRC_W - SRC_W // 5):
        y = first_opaque(px, x, thr)
        if 0 <= y <= EXPECT_MAX_INSET * 2:
            insets.append(y)
    p(f"源纸        : {SRC.name} {paper.size}   alpha 最大 {amax}   实心阈值 >={thr}")
    p(f"顶边中段内缩 : min={min(insets)} max={max(insets)} 众数≈"
      f"{sorted(insets)[len(insets) // 2]} 源 px"
      f"（= {min(insets) / 2:.1f}~{max(insets) / 2:.1f} CSS px）")
    p(f"  采样范围   : x ∈ [{SRC_W // 5}, {SRC_W - SRC_W // 5}]（避开四角）")
    p(f"  断言区间   : [{EXPECT_MIN_INSET}, {EXPECT_MAX_INSET}]")
    if not (EXPECT_MIN_INSET <= min(insets) and max(insets) <= EXPECT_MAX_INSET):
        sys.exit(
            f"顶边内缩 {min(insets)}~{max(insets)} 不在断言区间 "
            f"[{EXPECT_MIN_INSET}, {EXPECT_MAX_INSET}] 内 —— "
            f"源位图可能换了。**先重新量**（ui/scripts/_tearprobe.py），"
            f"再决定 slice 能不能还是 {SLICE}。"
        )
    if max(insets) >= SLICE:
        sys.exit(
            f"最深撕口 {max(insets)} 源 px ≥ slice {SLICE} —— "
            f"渐变会被九宫格截断，接缝会露出半透明硬边。调大 SLICE。"
        )
    p("  ⇒ 断言通过：源位图没换，slice 够深")
    p("")

    # ── 2. 裁四条真边 ───────────────────────────────────────────
    #
    # ⚠⚠⚠ **带子必须「跨过」撕口，不能停在撕口前**（这一版修的就是它）。
    #
    # 上一版写的是：
    #     top_band = paper.crop((0, mid_top_y - band, SRC_W, mid_top_y))
    # 而 `mid_top_y` 是**第一个不透明像素**（撕口外沿），
    # 于是这条带子取的是 `[mid_top_y - 32, mid_top_y)` ——
    # **整条都在撕口外侧，全透明**。
    # 再 `NEAREST` 压到 32 像素宽，撕口被挤在最右边 8 像素里，
    # 24/32 个位置画出来什么也没有。
    # ⇒ 屏幕上看到的是「边没了，只剩一条 box-shadow 的直线」。
    #   那条**直线不是撕口，是撕口缺席后剩下的投影** ——
    #   而投影是方的，于是读成框。
    #
    # ⚠ `_stripcheck.py` 量到了这个（实测）：
    #     顶边深度序列 = [-1]×24 + [30,29,28,26,24,24,23,23]
    #     底边         = [-1]×24 + [31,30,30,30,29,29,29,29]
    #     左/右边同理，各只有 8~11/32 个位置有纸
    #   **四条边的撕口都被挤到末端 8px**，中间 24px 是空的。
    #
    # ⇒ 正确取法：**从画布外沿切到撕口内侧**，让整条带子的开头就是撕口，
    #   后面跟着实心纸。CSS 的九宫格只取 `border-width` 那一段，
    #   所以带子里多余的纸会被丢掉，不影响。
    corner = SLICE
    band = SLICE

    img = Image.new("RGBA", (SIDE, SIDE), (0, 0, 0, 0))

    # 纸芯：取**没有撕口也没有笔迹**的干净区（`_tearprobe.py` 实测那里是
    # rgb(249,235,197) 一类的均匀纸色）。**不提亮** —— 提亮 4% 会让纸条
    # 整块比纸还亮，读成「被选中的卡片」，那是 §4.2b 否掉的色块语言。
    # 纸与纸的分离交给 CSS 的 box-shadow。
    img.paste(paper.crop((320, 500, 320 + SIDE, 500 + SIDE)), (corner, corner))

    # 四条边的**起点**：撕口外沿（= 第一个实心像素）。
    # 下面每条带子都从这里开始往纸里切 band 像素。
    def edge_start() -> dict[str, int]:
        top = next(y for y in range(SRC_H) if px[SRC_W // 2, y][3] >= thr)
        bot = next(y for y in range(SRC_H - 1, -1, -1) if px[SRC_W // 2, y][3] >= thr)
        lef = next(x for x in range(SRC_W) if px[x, SRC_H // 2][3] >= thr)
        rig = next(x for x in range(SRC_W - 1, -1, -1) if px[x, SRC_H // 2][3] >= thr)
        return {"top": top, "bottom": bot, "left": lef, "right": rig}

    st = edge_start()
    p(f"  四条边的撕口外沿: 顶 y={st['top']}  底 y={st['bottom']}  "
      f"左 x={st['left']}  右 x={st['right']}")

    # 上下中段：从**画布上沿**切到撕口再往里 band 像素。
    # ⚠ 起点是 y=0 而不是 st['top'] ——
    #   从 y=0 切才能把「全透明的外侧」一起带上，
    #   那段透明正是我们要让它溢出布局框的东西（见 PaperPanel 的 `margin: -8px`）。
    #   从 st['top'] 起切会丢掉外侧的透明，
    #   于是「透明外沿」不存在，负边距就会真的切到纸芯。
    top_band = paper.crop((0, 0, SRC_W, st["top"] + band))
    bot_band = paper.crop((0, st["bottom"] - band, SRC_W, SRC_H))
    lband = paper.crop((0, 0, st["left"] + band, SRC_H))
    rband = paper.crop((st["right"] - band, 0, SRC_W, SRC_H))

    # ⚠⚠ **缩放的方向反了才是关键。** 九宫格的边要**沿边不变、垂直边伸缩**：
    #     顶边那条必须**沿 x 保持 1:1**（撕口的参差是横向的），
    #     只在**纵向**（垂直于边的方向）压成 band 像素。
    #   上一版写 `top_band.resize((SIDE, band))` 看起来对了，
    #   但它的**源**已经是错的（整条透明），所以无论怎么缩都不对。
    #   现在源对了：`resize((SIDE, band))` 就是对的 ——
    #   x 方向 760 → 128（横向压缩 0.17）会让撕口参差被抹平，
    #   ⇒ 横向必须**取一段**而不是压全宽。
    #
    # ⚠ 正确做法：横向**只取 SIDE 宽的一段中段**，
    #   纵向压到 band。九宫格会把这段沿 x 重复铺开。
    # ⚠⚠⚠ **纵向不缩放**（这一版修的就是它）。
    #
    # 上一版写 `resize((SIDE, band), LANCZOS)` 把撕口从源深 19px 压成 12px。
    # 而**撕口的价值全在那条起伏的轮廓上**：
    # `_stripcompare.py` 的 NEAREST 4× 并排对比实测（左 = 面板，右 = strip）：
    # 面板那条是**参差不齐的纸边**，strip 那条是**几乎齐平、只有微弱起伏**的线。
    # ⇒ LANCZOS 把参差**平滑掉了**，那正是本文件开头引的「坑一」
    #   （周期抖动变成珠子链）同一个病：**重采样会抹掉撕口的统计特征**。
    #
    # ⇒ 正确做法：源带厚度就取 `band`，**四条边各从源图直接切**，
    #   一个像素都不缩放、也不翻转。
    # ⚠ **不许用 `transpose` 造底边/右边**：翻转后沿边方向反了，
    #   角上按 `CORNER_AT` 取样就会取到另一端 —— 而那一端正好是纸的角，
    #   撕口深达 24~52 源 px，超出 band ⇒ 角整块透明。
    def slice_top(im: Image.Image) -> Image.Image:
        cx = SRC_W // 2
        return im.crop((cx - SIDE // 2, 0, cx - SIDE // 2 + SIDE, band))

    def slice_bot(im: Image.Image) -> Image.Image:
        cx = SRC_W // 2
        # ⚠ 底边的「边沿」在**下**：从下往上取 band 像素
        return im.crop((cx - SIDE // 2, SRC_H - band, cx - SIDE // 2 + SIDE, SRC_H))

    def slice_left(im: Image.Image) -> Image.Image:
        cy = SRC_H // 2
        # ⚠ 竖边带的 (x, y) = (厚度, 沿边) ⇒ 输出尺寸 band × SIDE
        return im.crop((0, cy - SIDE // 2, band, cy - SIDE // 2 + SIDE))

    def slice_right(im: Image.Image) -> Image.Image:
        cy = SRC_H // 2
        return im.crop((SRC_W - band, cy - SIDE // 2, SRC_W, cy - SIDE // 2 + SIDE))

    top_mid = slice_top(paper)
    bot_mid = slice_bot(paper)
    left_mid = slice_left(paper)
    right_mid = slice_right(paper)

    # ── 四角：**由两条真撕边相交而成**，不能从纸的外角切 ──────────
    #
    # ⚠⚠⚠ **这一版修的就是角。** 上一版写的是
    #     img.paste(paper.crop((0, 0, corner, corner)), (0, 0))
    # 也就是从**纸自己的外角**切 32×32。而 `_tearprobe.py` 实测：
    #     顶角内缩 52 / 44 / 32 / 24 源 px，越靠角越深
    #     右下角更极端 —— 内缩 895 / 1022 源 px（447 / 511 CSS px）
    # ⇒ 那块 32×32 **整块都在透明带里**，贴上去等于把纸片的四个角撕没了。
    #   而它盖住的正是顶/左边带的前 32 列 ——
    #   于是 `_stripcheck.py` 量到「24/32 位置全透明」，
    #   屏幕上只剩下 box-shadow 的**直线**（用户红框标的那三条）。
    #
    # ## 角应该怎么来
    #
    # 从一整张纸上撕下一片，角上**两道撕口相交** ——
    # 所以角是「顶边那条撕口」与「左边那条撕口」的**交集**，不是纸的外角。
    # 而我们手上正好有这两条真撕口。
    #
    # ⚠⚠⚠ **角 = 取横边与竖边各自中段轮廓的 min**（踩了四轮才收敛，原因记在这）
    #
    #   ① 从**纸的外角**切 ⇒ 角上内缩 52/44/32/24、右下角 895/1022 源 px，
    #      整块在透明带里，把纸片四角撕没了，还盖掉顶/左边带的前 32 列
    #      ⇒ 只剩 box-shadow 的**直线**（用户红框标的那三条）。
    #   ② 从带的**端头**取 ⇒ 端头就是角，角已被撕得很深，取到全透明。
    #   ③ `transpose` 造底边/右边 ⇒ 沿边方向反了，取到另一端（也是角）。
    #   ④ 上面三种都绕不开同一个事实：**带的端头就是角，角已经撕没了。**
    #
    # ⇒ 唯一稳的做法是**沿边方向从中段取**，且两边都取同一段中段。
    #
    # ⚠⚠ **自检里角只查「靠内有纸」，不查「沿边连续」**
    #   （这是第四轮才想通的一条）：
    #   角区本来就该被咬掉一块，`min` 会让两边的深撕口在角上叠成更深的透明。
    #   那是**对**的。可一旦角上的透明超过 `band`，那一小块就会在
    #   渲染时退化成「边上一个方缺口」—— 而那比缺角更难看。
    #   ⇒ 角上**限制最深不超过 `band - 6`**：宁可角浅一点，
    #     也不要角上出现「穿透」的方洞。
    CORNER_AT = corner
    CORNER_MAX_DEPTH = band - 6   # 角上最多咬到这里，再深就成方洞了

    def build_corner(
        hb: Image.Image, vb: Image.Image, flip_x: bool, flip_y: bool
    ) -> Image.Image:
        """hb 横边带（SIDE 沿边 × band 厚）；vb 竖边带（band 厚 × SIDE 沿边）。"""
        out = Image.new("RGBA", (corner, corner))
        op = out.load()
        for y in range(corner):
            for x in range(corner):
                h = hb.getpixel(
                    (CORNER_AT + (corner - 1 - x if flip_x else x),
                     corner - 1 - y if flip_y else y)
                )
                # ⚠ 竖边带的分量顺序与横边**相反**：
                #   横边 (沿边, 厚度)，竖边 (厚度, 沿边)。
                #   这一版之前两次都栽在这里 —— 写对了 flip 逻辑，
                #   却把两个带当成同样的分量顺序，于是竖边那侧恒为 0。
                v = vb.getpixel(
                    (corner - 1 - y if flip_y else y,
                     CORNER_AT + (corner - 1 - x if flip_x else x))
                )
                op[x, y] = (h[0], h[1], h[2], min(h[3], v[3]))
        return out

    tl = build_corner(top_mid, left_mid, False, False)
    tr = build_corner(top_mid, right_mid, True, False)
    bl = build_corner(bot_mid, left_mid, False, True)
    br = build_corner(bot_mid, right_mid, True, True)

    # ⚠ **角上不让它穿透。** 把角区里比 `CORNER_MAX_DEPTH` 更深的部分
    #   补成实心纸 —— 代价是角不那么「咬」，换来的是
    #   渲染时不会在边上留下一个方缺口。
    # ⚠ 这么做算「合成像素」吗：**算，而且是明说的**。
    #   本文件开头的原则是「四条边不合成任何像素」，
    #   而这一处是例外：角是四边之外的第五个部件，
    #   它的正确形态取决于渲染时的 `stretch` 接缝，光靠采样给不出来。
    #   宁可一处有据可查的补齐，也不要一个说不清的方洞。
    for cimg in (tl, tr, bl, br):
        cp = cimg.load()
        for y in range(corner):
            for x in range(corner):
                a = cp[x, y][3]
                if a > 0 and a < 40 and (x + y) > CORNER_MAX_DEPTH:
                    # 太深 ⇒ 用同一行里最近的不透明像素补
                    for k in range(corner - 1, -1, -1):
                        if k <= CORNER_MAX_DEPTH and cp[x, k][3] >= 245:
                            cp[x, y] = (*cp[x, k][:3], 254)
                            break

    tl = build_corner(top_mid, left_mid, False, False)
    tr = build_corner(top_mid, right_mid, True, False)
    bl = build_corner(bot_mid, left_mid, False, True)
    br = build_corner(bot_mid, right_mid, True, True)

    # 先贴四条中段（条状），**再**贴四角盖住它们的端头。
    # ⚠ 顺序反了会被中段把角上撕口的参差盖掉 —— 那正是接缝露硬边的地方。
    img.paste(top_mid, (0, 0))
    img.paste(left_mid, (0, 0))
    img.paste(right_mid, (SIDE - band, 0))
    img.paste(bot_mid, (0, SIDE - band))

    # ⚠ **不做镜像/翻转**（`build_corner` 里的 flip 只是**取样方向**，
    #   不是把位图翻转 —— 位图仍然逐像素来自面板自己的撕口）。
    #   「四条边长得不一样」本来就是撕边的特征。
    img.paste(tl, (0, 0))
    img.paste(tr, (SIDE - corner, 0))
    img.paste(bl, (0, SIDE - corner))
    img.paste(br, (SIDE - corner, SIDE - corner))

    # ── 3. 自检：撕口的**形状**对不对 ─────────────────────────────
    #
    # ⚠⚠ **判据的第一版写错了，写的是「带内有纸的位置 ≥ 28/32」**，
    #   于是它把**外侧那段本来就该透明的**判成了缺陷。
    # 那段透明不是缺陷，它是**设计**：
    #   PaperPanel 的 `.cs-act { margin: -8px }` 靠它溢出布局框
    #   （否则三条纸的撕边共 96px，面板放不下）。
    # ⇒ 正确的判据是**形状**，不是「有多少纸」：
    #
    #   ① 透明必须**只出现在最外侧**，不能插在中间
    #      （中间插空 = 撕口被压扁成两截，接缝露硬边）
    #   ② 透明之后必须**紧接**一段爬升（alpha 0 → 254 的渐变）
    #   ③ 深度要**起伏**（≥3 种取值），齐平就是直线不是撕口
    def alpha_profile(fn_axis: str, i: int) -> list[int]:
        """返回该位置从边沿往里 32 个像素的 alpha。"""
        out = []
        for k in range(band):
            if fn_axis == "top":
                out.append(px_s[i, k][3])
            elif fn_axis == "bottom":
                out.append(px_s[i, SIDE - 1 - k][3])
            elif fn_axis == "left":
                out.append(px_s[k, i][3])
            else:
                out.append(px_s[SIDE - 1 - k, i][3])
        return out

    def check(name: str, axis: str, prof_first: list[int]) -> bool:
        ok = True
        # ⚠⚠ **只查中段**（角区本来就该透明：角上两道撕口相交，咬得最深）。
        lo_i, hi_i = band, SIDE - band
        ramp_cols = 0
        hard_cols = 0
        for i in range(lo_i, hi_i):
            prof = alpha_profile(axis, i)
            first = next((k for k, a in enumerate(prof) if a >= 200), None)
            if first is None:
                p(f"  ❌ {name} 中段位置 {i}: **整条带全透明**")
                ok = False
                continue
            # ① 中间不能插空
            if any(a == 0 for a in prof[first:]):
                p(f"  ❌ {name} 中段位置 {i}: 撕口**中间**有空段 —— 接缝会露硬边")
                ok = False
                continue
            # ② 这一列有没有渐变
            ramp = prof[max(0, first - 5):first + 1]
            if any(20 < a < 235 for a in ramp):
                ramp_cols += 1
            else:
                hard_cols += 1
        # ⚠⚠ **判据是「有多少列有渐变」，不是「每一列都要有渐变」。**
        #   上一版逐列要求渐变，于是报出十几条「硬边」——
        #   而那些列在**面板自己的位图上本来就没有渐变**
        #   （撕口有的地方纤维整根崩掉，直接从透明跳到实心）。
        #   ⇒ 那是真撕口的正常形态，被我判成了缺陷。
        #   判据改成：带内**整体**必须既有渐变列也有硬边列
        #   （全渐变 = 被 LANCZOS 平滑过；全硬边 = 没撕口）。
        n = hi_i - lo_i
        p(f"    渐变列 {ramp_cols}/{n}   硬跳列 {hard_cols}/{n}")
        # ⚠⚠ **没有绝对阈值** —— 每条边的「齐」程度不同，
        #   而那条基准是**面板自己的边**（`_tearprobe.py` 实测）：
        #     顶边内缩 12~25（众数 21）  起伏明显
        #     左边内缩 7~21            起伏明显
        #     **底边内缩 20~31（众数 28）  面板自己就很齐**
        #   ⇒ 「渐变列 63/64」对底边是**面板同款**，不是缺陷。
        #   上一版用 `> 0.95` 判死，于是把底边判成「均匀的斜坡」。
        #   ⇒ 这里只拦**真正平滑掉**的那一种：
        #     深度取值 ≤ 2 种（完全齐）或渐变列 == n（连硬跳都没有）。
        # ⚠ `uniq` 必须在这里重算 —— 它在调用方算的，
        #   而 `check()` 拿不到那个作用域（第一版直接引用报
        #   `TypeError: '<=' not supported between 'list' and 'int'`，
        #   脚本崩在自检里，报告还留在上一轮 ⇒ 看到的是**旧结论**）。
        uniq = len({d for d in prof_first if d >= 0})
        if uniq <= 2:
            p(f"    ❌ 深度只有 {uniq} 种取值 —— 完全齐平，那是直线不是撕口")
            ok = False
        if ramp_cols == n and hard_cols == 0:
            p(f"    ❌ 没有一处硬跳 —— 撕口被平滑掉了")
            ok = False
        return ok

    px_s = img.load()
    p("")
    p(f"── 自检：四条边的撕口形状（中段 [{band}, {SIDE - band}) 查）")
    p(f"   判据：撕口不许缺席 / 中间不许插空 / 渐变不许断 / 深度要起伏")
    p(f"   ⚠ 角区不查 —— 角上两道撕口相交，本来就是最深的透明区")
    all_ok = True
    for name, axis in (("顶边", "top"), ("底边", "bottom"),
                       ("左边", "left"), ("右边", "right")):
        mid = range(band, SIDE - band)
        prof_first = [
            next((k for k, a in enumerate(alpha_profile(axis, i)) if a >= 200), -1)
            for i in mid
        ]
        depths = [d for d in prof_first if d >= 0]
        uniq = sorted(set(depths))
        p(f"  {name} 中段: 撕口起点 {min(depths)}~{max(depths)} 源 px，"
          f"{len(uniq)} 种取值")
        if not check(name, axis, prof_first):
            all_ok = False

    # 角：**只查「不能整块穿透」**（第四轮才收敛的判据）。
    # ⚠⚠ 前三轮都在这里判错，两次方向各错一次：
    #   ① 沿对角线查 ⇒ 对角线是两边最深处相交处，整条都透明 ⇒ 误判
    #   ② 沿边沿那一行查 ⇒ 边沿本来就该透明 ⇒ 误判
    #   ③ 沿「离边沿 4 px」切面查「要爬到实心」
    #      ⇒ 角区被 `min` 咬深是**设计**，而 corner 只有 32px，
    #         咬到 20px 就已经是「角上缺一块」⇒ 判据与设计对撞。
    # ⇒ 真正要拦的只有一种：**角区整块透明**（那会让纸片四角消失，
    #   渲染成一个方缺口）。
    p("")
    p("── 自检：四角只需「不整块穿透」（角被咬掉是设计）")
    for cn, (ox, oy) in {
        "左上": (0, 0),
        "右上": (SIDE - 1, 0),
        "左下": (0, SIDE - 1),
        "右下": (SIDE - 1, SIDE - 1),
    }.items():
        opaque = sum(
            1 for y in range(corner) for x in range(corner)
            if px_s[ox + (x if ox == 0 else -x),
                    oy + (y if oy == 0 else -y)][3] >= 245
        )
        ratio = opaque / (corner * corner)
        ok = ratio >= 0.45
        p(f"  {cn}: 不透明像素 {opaque}/{corner * corner} = {ratio:.0%}  "
          f"{'✅' if ok else '❌ 角区几乎全透明 —— 渲染会出方缺口'}")
        if not ok:
            all_ok = False

    if not all_ok:
        fail(
            "四条边里有不合格的 —— 撕口形状不对。\n"
            "**自检在写文件之前，所以 static/strip.png 保持原样，绝不写回坏图。**"
        )
    p("  ⇒ ✅ 四条边的撕口形状都合格")

    OUT.parent.mkdir(parents=True, exist_ok=True)
    img.save(OUT)

    p("")
    p("四条边来源  : 全部裁自面板自己的位图，**未合成任何像素**")
    p(f"  撕口外沿    顶 y={st['top']}  底 y={st['bottom']}  "
      f"左 x={st['left']}  右 x={st['right']}")
    p(f"纸条        : {OUT.name} {img.size}（slice {SLICE} 源 px = {SLICE // 2} CSS px）")
    p("")
    p(f"⚠ CSS 那边必须写：border-image: url('/strip.png') {SLICE} fill / "
      f"{SLICE // 2}px / 0 stretch")
    p(f"  border-width 也要 = {SLICE // 2}px")
    p("")
    p(f"⚠ 排版：撕边 {SLICE // 2}px × 2 = {SLICE}px / 条，三条 = {3 * SLICE}px。")
    p(f"  但 `margin: -8px` 让**最外侧那段透明**溢出布局框"
      f"（PaperPanel 的 `.cs-act`），")
    p(f"  所以实占只有 `{SLICE} - 8 = {SLICE - 8}px` / 条。")
    p(f"  ⚠ 排版结论**不要在本脚本里算** —— 用 `ui/static/budget.html` 量。")
    p(f"    本脚本只管撕边对不对。")

    REPORT.parent.mkdir(parents=True, exist_ok=True)
    write_report()
    print(f"written: {REPORT}")


if __name__ == "__main__":
    build()
