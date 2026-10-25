"""红铅笔笔迹生成器。

⚠ 这个文件是 `ui/src/lib/marks.js` 的**唯一来源**，而那个文件是入库的。
   所以它也必须入库——曾经放在 `.scratch/tmp-probe/` 下，而那个目录被
   gitignore，于是产物入库、来源没入库，`marks.js` 从此不可复现。

用法:
    python ui/scripts/pencilgen.py ui/src/lib/marks.js

为什么要生成器：手写 SVG path 的坐标只能靠试，而我连试错都判不准
——第一轮两条 ribbon 的**收放方向写反了**（上撇远端最粗、底角最细，
和参考恰好相反），整页图看不出，是 6× 真像素才暴露的。

改成：**中心线 + 宽度剖面** → 程序算轮廓。宽度剖面是唯一需要手调的量，
而它是**能对着照片量的量**。

## 两次踩到的坑

1. **收放方向反了**（第一轮）。手写坐标时把上撇的宽度曲线搞反。
   → 所以宽度必须是**剖面函数**，一眼能看出哪头粗。

2. **横纵比错了**（第二轮）。生成出来的勾量下来是 50.6 × 19.5
   ≈ **2.6 : 1**，而照片里的勾接近 **1.17 : 1**——**几乎和它一样高**。
   笔法再准，比例不对就读不出「红铅笔」，只读得出「一条扁扁的折线」。
   → 所以横纵比也要量，而且必须量。

宽度剖面与横纵比的依据：用户 2026-10-12 提供的真实红铅笔照片，
放大件见 `target/probe/ref_tick_2x.png` / `ref_ring_2x.png` / `ref_cross_2x.png`。
"""
import math
import sys

# ⚠ Windows 上 Python 的 stdout 默认是传统代码页（GitHub 的 Windows runner
#   上就是 cp1252），一句中文 print 就 UnicodeEncodeError 带栈回溯崩掉。
#   本仓在 scripts/fetch_models.py 上踩过（CI run 39037571268），
#   而 ocrbench 这一族当时**一个都没加**——守卫的不变量只管
#   「gate.ps1 会 python 调起的脚本」，人工跑的台架在射程外。
#   不能指望调用方设 PYTHONIOENCODING，必须在脚本里自己改。
#   errors="replace"：真有编不出来的字符时**降级成 ? 而不是崩**——
#   台架的失败模式必须是「读数不符」，不是「打印失败 ⇒ 栈回溯」。
#
# ⚠⚠ **2026-10-25：这里原本缺 `import sys`**，脚本在自己第 40 行就用
#   `sys.stdout` ⇒ **一运行就 NameError**。
#   ⇒ 它从来没被真正执行过——门禁里**没有**「重生成 marks.js 再比对」这一步，
#   所以产物与来源是否同步，**没有任何机器在管**。
#   ⇒ 这正是「从未被执行过的分支，不等于它是对的」那条纪律的形态：
#   一个只写不跑的脚本，坏了也没人知道。
for _stream in (sys.stdout, sys.stderr):
    if hasattr(_stream, "reconfigure"):        # 3.7+ 都有
        _stream.reconfigure(encoding="utf-8", errors="replace")

#  三支共用一个 60 × 40 的取景框（1 单位 = 1 px @ 1×）。
VIEW_W, VIEW_H = 60, 40


def catmull_rom(pts, samples_per_seg=14):
    """Catmull-Rom 插值，让中心线是条顺滑的曲线而不是折线。"""
    if len(pts) < 2:
        return list(pts)
    p = [pts[0]] + list(pts) + [pts[-1]]
    out = []
    for i in range(len(p) - 3):
        p0, p1, p2, p3 = p[i], p[i + 1], p[i + 2], p[i + 3]
        for s in range(samples_per_seg):
            t = s / samples_per_seg
            t2, t3 = t * t, t * t * t
            x = 0.5 * ((2 * p1[0]) + (-p0[0] + p2[0]) * t
                       + (2 * p0[0] - 5 * p1[0] + 4 * p2[0] - p3[0]) * t2
                       + (-p0[0] + 3 * p1[0] - 3 * p2[0] + p3[0]) * t3)
            y = 0.5 * ((2 * p1[1]) + (-p0[1] + p2[1]) * t
                       + (2 * p0[1] - 5 * p1[1] + 4 * p2[1] - p3[1]) * t2
                       + (-p0[1] + 3 * p1[1] - 3 * p2[1] + p3[1]) * t3)
            out.append((x, y))
    out.append(tuple(pts[-1]))
    return out


def ribbon(pts, widths, smooth=14):
    """中心线 + 每点宽度 → 闭合的填充轮廓。"""
    if callable(widths):
        n = max(2, len(pts))
        widths = [widths(i / (n - 1)) for i in range(n)]
    if len(widths) != len(pts):
        raise ValueError("widths must match pts")

    line = catmull_rom(pts, smooth)
    n = len(pts)
    step = max(1, (len(line) - 1) // (n - 1))
    ws = []
    for i in range(len(line)):
        seg = min(n - 2, i // step)
        f = min(1.0, (i - seg * step) / step)
        ws.append(widths[seg] * (1 - f) + widths[seg + 1] * f)

    left, right = [], []
    for i, (x, y) in enumerate(line):
        if i == 0:
            dx, dy = line[1][0] - x, line[1][1] - y
        elif i == len(line) - 1:
            dx, dy = x - line[-2][0], y - line[-2][1]
        else:
            dx = line[i + 1][0] - line[i - 1][0]
            dy = line[i + 1][1] - line[i - 1][1]
        L = math.hypot(dx, dy) or 1.0
        nx, ny = -dy / L, dx / L
        h = ws[i] / 2.0
        left.append((x + nx * h, y + ny * h))
        right.append((x - nx * h, y - ny * h))

    def f(v):
        # 一位小数：viewBox 1 单位 = 1 px，亚像素精度在这里是浪费
        return f"{v:.1f}".rstrip("0").rstrip(".")

    d = [f"M{f(left[0][0])} {f(left[0][1])}"]
    for x, y in left[1:]:
        d.append(f"L{f(x)} {f(y)}")
    for x, y in reversed(right):
        d.append(f"L{f(x)} {f(y)}")
    d.append("Z")
    return "".join(d)


# ── 勾 ────────────────────────────────────────────────────────────
#  单条中心线走完「落笔 → 底角 → 右上扬起」。底角的堆积是过渡
#  出来的，不是两个图形叠出来的硬结。
#
#  第四轮踩到的坑：**几何本身也是错的，不是只有表面不对**。
#  把参考勾归一到同一外框再和生成版并排（`target/probe/ref_vs_gen.png`），
#  量出来两个硬指标都差约 1.6 倍：
#
#      笔宽 / 勾高   参考 0.10 ~ 0.12    旧生成版 0.17   ← 粗了近一倍
#      勾宽 / 勾高   参考 0.75 ~ 0.97    旧生成版 1.48   ← 宽了近一倍
#
#  上一轮把横纵比写成 1.48 时留了一句「小尺寸下偏高会显得笨」的余量——
#  方向就搞反了：参考的勾本来就**比看起来更窄**。
#
#  另一个从放大图才看得出的特征：**落笔处有一个很短的挑脚**。
#  参考里几乎每一笔起笔都先向左下带出 2~3px 再压下去，少了它就只是
#  一条折线。底角也因此从「圆钝」变成「尖」。
TICK_LINE = [
    (13.5, 10.0),   # 落笔，最重
    (15.8, 8.4),    # 极短的挑脚——手腕带出来的，不是设计出来的
    (17.6, 11.8),   # 转入下压
    (19.8, 20.0),
    (21.6, 29.0),
    (23.0, 35.2),   # 底角：尖
    (26.2, 31.6),   # 转为上撇
    (30.6, 25.8),
    (35.0, 19.0),
    (38.8, 12.4),
    (41.4, 5.6),    # 收笔，尖
]


def tick_width(t):
    """落笔重 → 底角堆积 → 一路收到尖。0.42 是底角的位置。"""
    if t < 0.30:                      # 下行笔：逐渐收细
        return 3.3 + (2.7 - 3.3) * (t / 0.30)
    if t < 0.42:                      # 底角：堆积
        u = (t - 0.30) / 0.12
        return 2.7 + (2.9 - 2.7) * u
    u = (t - 0.42) / 0.58            # 上撇：一路收到尖
    return 2.9 + (0.4 - 2.9) * (u ** 0.62)


# ── 圈 ────────────────────────────────────────────────────────────
#  照片里圈没有一根是单笔，而且**不是一道粗环，是 2~4 道细线叠画**：
#  每道都有自己的疏密与出头，整体才像手画的椭圆而不是印刷的 O。
#  所以这里给**三道**，每道宽度还各不相同。
#
#  横纵比量过：参考多数在 0.87 ~ 1.34，中位约 1.06。
RING_LINE = [
    (22.5, 25.0), (21.0, 14.5), (25.5, 6.0), (33.5, 4.2),
    (43.0, 7.0), (48.5, 15.5), (47.0, 25.5), (39.5, 33.5),
    (29.5, 35.0), (20.5, 28.5), (18.0, 22.0), (17.5, 27.0),
]


def ring_width(t):
    """单道很细，起收略重——参考里每一道都是这个脾气。"""
    return 1.15 + 0.35 * abs(2 * t - 1)


#  三道：(偏移, 宽度倍率)。倍率不齐，所以叠起来才不是「一个粗环」。
RING_PASSES = [
    ((0.0, 0.0), 1.00),
    ((1.7, -1.4), 1.20),
    ((-1.4, 1.3), 0.85),
]


# ── 圈（圈住字）────────────────────────────────────────────────
#  独立一个圈只是「这里有一个」，语义偏弱；**圈住档位名**才是
#  「这一档被圈中了」。所以要一个横长的环，包住 11px 的字。
#  没有照片可依，宽度按勾收细后的同一支笔来定。
RING_LABEL_LINE = [
    (30.0, 21.0), (10.0, 20.4), (5.5, 16.5), (8.0, 12.4),
    (17.0, 10.6), (32.0, 10.2), (46.0, 10.6), (52.5, 12.8),
    (54.5, 17.0), (51.0, 21.2), (42.0, 23.4), (30.0, 23.8),
    (18.0, 23.4), (11.0, 22.6), (8.5, 20.0), (12.0, 17.6),
]


def ring_label_width(t):
    """比独立圈粗一点——它要隔着空隙去包住字，太轻圈不住。"""
    return 1.35 + 0.4 * abs(2 * t - 1)


RING_LABEL_PASSES = [
    ((0.0, 0.0), 1.00),
    ((1.4, -1.1), 1.15),
]


# ── 幅面档位（2026-10-13）─────────────────────────────────────────
#
#  两支笔都按「同一支笔放大」处理：中心线缩放，**笔重剖面原样不动**。
#  改的是笔画画多大，不是笔多重——放大了笔也跟着粗就成了一块墨饼。
#
#  实测（解析 marks.js 生成的 path 坐标 + 6× 真像素，不是估的）：
#      档位名「只填入」排版宽 33.66px（CJK 全角 11px + 0.02em 字距 ×3），
#      行高 11px（`.cs-slot-name` 的 font-size/line-height 都是 11）
#
#  原始的圈是 52.10 × 16.40，套在 11px 的字上：横向每侧还空 9.2px，
#  纵向只空 2.7px —— 扁椭圆的上沿直接压在字头上，读成「圈没画开」
#  而不是「圈大」。放到 74.20 × 28.60 之后才读成「盖了一个章」。
#  💭 这里的判据是**圈高与字高的比**：小于约 1.5 倍读成「字的边框」，
#  超过约 2.5 倍才读成「印章」。1.66 倍正好卡在中间，所以两头都不像。
def _scale_centerline(pts, kx, ky):
    """绕自身外框中心缩放中心线。手抖的形状原样保留，只改幅面。"""
    xs = [p[0] for p in pts]
    ys = [p[1] for p in pts]
    cx = (min(xs) + max(xs)) / 2
    cy = (min(ys) + max(ys)) / 2
    return [(cx + (x - cx) * kx, cy + (y - cy) * ky) for x, y in pts]


def _recenter(pts, w, h):
    """把中心线摆到 (w/2, h/2)。取景框变了，笔心就得重新对回框心，
    否则页面上按笔心反算出来的偏移量全错。"""
    xs = [p[0] for p in pts]
    ys = [p[1] for p in pts]
    dx = w / 2 - (min(xs) + max(xs)) / 2
    dy = h / 2 - (min(ys) + max(ys)) / 2
    return [(x + dx, y + dy) for x, y in pts]


#  ⚠ 取景框必须跟着放大：根 `<svg>` 的 overflow 是 hidden，画到 60×40
#    之外会被**静默裁掉**——症状是「代码改了、图没变」，那正是量具
#    自己骗自己。所以放大的档位各自配一个更大的 VIEW。
RING_VIEW = {"w": 84, "h": 52}
TICK_VIEW = {"w": 56, "h": 34}

#  圈：纵向比横向放得多，让扁椭圆的长宽比往字靠
RING_LABEL_LINE_BIG = _scale_centerline(RING_LABEL_LINE, 1.10, 1.46)  # → 约 57 × 22.6
RING_LABEL_LINE_HUGE = _recenter(
    _scale_centerline(RING_LABEL_LINE, 1.45, 1.90), RING_VIEW["w"], RING_VIEW["h"]
)  # → 约 74 × 28.6

#  勾：**加宽压扁**，不是等比放大。2026-10-13 用户判「纵向过长」。
#  💭 病根不是「高」是**窄**：三个字排版宽 33.66px，base 勾只有 29.10px
#    宽——盖不住词就只剩一竖；等比放到 44.5 × 47.1 更糟，宽还是不够而
#    高已 4.3 倍字高，读成「一竖划了个钩」。圈那边踩过同一个坑
#    （RING_LABEL_LINE），判据是**盖不盖得住词**。故两档都 kx 上 ky 下，
#    外廓落在「宽过词、矮于圈」这一带。
TICK_LINE_MID = _recenter(
    _scale_centerline(TICK_LINE, 1.375, 0.909), TICK_VIEW["w"], TICK_VIEW["h"]
)  # → 约 40 × 28
TICK_LINE_BIG = _recenter(
    _scale_centerline(TICK_LINE, 1.718, 0.844), TICK_VIEW["w"], TICK_VIEW["h"]
)  # → 约 50 × 26


# ── 叉 ────────────────────────────────────────────────────────────
CROSS_A = [(12.0, 7.5), (19.0, 14.0), (27.0, 21.5), (36.0, 28.5), (42.0, 32.5)]
CROSS_B = [(42.0, 7.5), (35.0, 14.5), (27.5, 22.0), (19.0, 29.0), (13.0, 32.5)]


def cross_width(t):
    """两笔都是中间略重、两端收锋。"""
    return 0.7 + 2.7 * (1.0 - abs(2 * t - 1) ** 1.5)


# ── 嵌套羽化 ────────────────────────────────────────────────────
#  第三轮踩到的坑：**叠笔偏移产生不了羽化**。
#
#  材质台 P1/P2/P3 三档在 6× 真像素下肉眼没差别——因为把同一张轮廓
#  偏移几份再叠上去，外层那一道的**最外缘仍然是满 opacity 的硬边**，
#  叠的结果只是把笔画**变粗**，边缘还多出几级台阶，读作毛刺而非石墨。
#
#  羽化要求密度**从中心往外逐级下降**，所以要的是**嵌套**：同一中心线，
#  宽度剖面按比例收窄，一层套一层。cumulative opacity（`1 - Π(1-o)`）：
#
#     带   宽度系数  本层 o   累计
#     0    1.00     0.22     0.22
#     1    0.86     0.24     0.41
#     2    0.70     0.44     0.67
#     3    0.52     0.52     0.84
#     4    0.30     0.58     0.93
#
# ⚠ 芯部三层的 o 是 2026-10-12 调过的，别退回 0.26/0.30/0.36。
#   起因是材质台 B 段比「圈还是勾」：圈的芯密度是 `1-0.38**3 = 0.945`，
#   而勾当时只有 `1-0.78*0.76*0.74*0.70*0.64 = 0.80`，差 1.18 倍。
#   在面板真实尺寸下（1:1 看，不是 3× 放大）那一笔就细成一根发丝，
#   读不出「当前档」。所以**只把芯部三层抬到与圈齐平**，外两层 0.22/0.24
#   原样不动——抬外层会毁掉「芯重边虚」，那正是嵌套存在的理由。
#   抬到 0.93 之后：羽化梯度保留，几何（宽度系数、横纵比、笔宽剖面）
#   一个字节没动，与照片的校准关系不受影响。
#
#  这才是「芯重边虚」。另外注意最内层最细：宽度剖面在收笔处本来就收到
#  0.6，×0.30 只有 0.18px，所以**尖端自动淡到看不见**——真实铅笔尖也是
#  这样，不用额外处理。
NEST = [
    (1.00, 0.22),
    (0.86, 0.24),
    (0.70, 0.44),
    (0.52, 0.52),
    (0.30, 0.58),
]


def nest(line, width_fn):
    """同一中心线，宽度剖面逐级收窄，产出嵌套轮廓。"""
    out = []
    for scale, opacity in NEST:
        out.append((ribbon(line, lambda t, s=scale: width_fn(t) * s), opacity))
    return out


def pass_layers(line, width_fn, passes):
    """多道叠画。圈的羽化**只能**来自这里，不能来自嵌套。

    ⚠ 圈的单道只有 1.15~1.5px，套 ×0.30 的嵌套等于 0.35px——看不见。
    笔迹足够粗才谈得上「芯 / 边」之分；细线的柔边是叠出来的，不是缩出来的。
    """
    out = []
    for (dx, dy), mul in passes:
        shifted = [(x + dx, y + dy) for x, y in line]
        out.append((ribbon(shifted, lambda t, m=mul: width_fn(t) * m), 0.62))
    return out


def nested_block(layers):
    """把 [(path, opacity)] 渲染成 JS 数组字面量。

    ⚠ 元素里**不能**自带尾逗号：调用处已经用 ",\\n" join 了，多一个逗号
    会让字面量变成**稀疏数组**（长度翻倍、隔一个空一个），症状是运行时
    `layer.d` 读到 undefined——页面直接白屏，而不是安静地少一层。
    """
    rows = ",\n".join(
        f"  {{ d: '{d}', o: {o:.2f} }}" for d, o in layers
    )
    return f"[\n{rows}\n]"


# ── 置信度短划（2026-10-25）────────────────────────────────────────
#
# ## 它是什么
#
# 面板那一格现在**不显示百分数**了（用户定案：首选行只有勾，
# 备选行改用一道**笔迹轻重**表示置信程度）⇒ 需要一支「短划」。
#
# ⚠ **三档只有笔重不同，中心线完全一样**——这是这个方案的全部含义：
# 「同一道批注，按得重一点还是轻一点」。道数、长度、位置都不参与编码。
#
# ## 为什么是三道叠画，而不是单道
#
# marks.js 的纪律：**细线没有芯边之分，它的柔边是叠出来的，不是缩出来的**
# （`TICK_NESTED` 那套嵌套只给粗笔迹）。一道短划正是细笔迹
# ⇒ 用 [`pass_layers`] 的路子：几道细线紧挨着叠，边缘呈阶梯状毛边。
#
# ⚠ 道间距**小于**道宽 ⇒ 三道是**叠**在一起的一道，不是三条分开的线；
# 间距随档位一起缩 ⇒ 笔越重，划痕越宽（道间距也占宽度）。
#
# ## 三档的宽度是量出来的，不是估的
#
# 面板上那个槽是 **26×16 px**，取景框 56×34 ⇒ 缩放 0.464。
# 判据是**真实像素下的墨迹厚度**：高档必须明显粗、低档必须**看得见**
# （上一轮实测：第一版低档 1.3 单位 = 0.6px ⇒ 淡到等于没有 ⇒ 那一档失效）。
#
CONF_LINE = [
    (9.0, 19.8),
    (17.0, 17.6),
    (26.0, 18.8),
    (35.0, 16.4),
    (47.0, 15.6),
]


def conf_width(t):
    """一道随手划的短痕：落笔稍重 → 中段稳 → 收锋。基准（高档）。"""
    if t < 0.18:
        return 3.6 + (3.2 - 3.6) * (t / 0.18)
    u = (t - 0.18) / 0.82
    return 3.2 + (0.9 - 3.2) * (u ** 0.7)


# 三档：(笔重倍率, 道间距倍率)。**间距跟着笔重一起缩**——
# 否则高档变粗而间距不变，划痕的「宽度」就跟不上笔重。
CONF_TIERS = {
    "high": (1.00, 1.00),
    "mid": (0.71, 0.74),
    "low": (0.55, 0.56),
}
# 基准笔宽（高档的单道笔宽，取景框单位）与基准道间距。
CONF_BASE_WIDTH = 3.6
CONF_BASE_GAP = 0.9
# 三道各自的错位（沿笔划方向为主，垂直方向给一点自然的参差）。
CONF_PASS_DX = (-0.9, 0.0, 0.9)
CONF_PASS_DY = (-0.25, 0.30, -0.20)
# 每道的 opacity。累计 = 1-(1-0.46)³ = 0.843 ⇒ 三档**一样浓**，
# 差别全在笔重（这正是「同一道批注，轻按 / 重按」）。
# ⚠ 2026-10-25 实测两次后从 0.34 提到 0.46：
#   · 笔重小的档位**覆盖面积也小**，同样的透明度下低档看着比中档淡得多；
#   · 1:1 实拍（真机 100% 缩放）里低档那道几乎看不见。
#   ⇒ 墨色一起提，低档才不会「等于没有」。
#   ⚠ 三档**同步**改 ⇒ 档间差异仍然只由笔重决定（墨色不参与编码）。
CONF_PASS_O = 0.46


def conf_layers(tier):
    """一档短划 = 三道叠画的细线。"""
    w_mul, gap_mul = CONF_TIERS[tier]
    out = []
    for dx, dy in zip(CONF_PASS_DX, CONF_PASS_DY):
        shifted = [(x + dx * gap_mul, y + dy * gap_mul) for x, y in CONF_LINE]
        out.append(
            (ribbon(shifted, lambda t, m=w_mul: conf_width(t) * m), CONF_PASS_O)
        )
    return out


def bbox(path_pts):
    xs = [p[0] for p in path_pts]
    ys = [p[1] for p in path_pts]
    return max(xs) - min(xs), max(ys) - min(ys)


def emit_js(path):
    """把笔迹写成一个 JS 模块。

    ⚠ 为什么是**生成的 JS**而不是手写 SVG path：
      1. 坐标是算出来的（中心线 + 宽度剖面），手抄必然抄错；
      2. 画布和新面板必须引用**同一份**几何，否则两边会长得不一样，
         而「档位标记长什么样」只有一个答案。
    """
    tick = ribbon(TICK_LINE, tick_width)
    cross_a = ribbon(CROSS_A, cross_width)
    cross_b = ribbon(CROSS_B, cross_width)

    tick_n = nest(TICK_LINE, tick_width)
    tick_mid = nest(TICK_LINE_MID, tick_width)
    tick_big = nest(TICK_LINE_BIG, tick_width)
    ring_m = pass_layers(RING_LINE, ring_width, RING_PASSES)
    ring_l_m = pass_layers(RING_LABEL_LINE, ring_label_width, RING_LABEL_PASSES)
    ring_l_big = pass_layers(RING_LABEL_LINE_BIG, ring_label_width, RING_LABEL_PASSES)
    ring_l_huge = pass_layers(RING_LABEL_LINE_HUGE, ring_label_width, RING_LABEL_PASSES)
    conf_strokes = {t: nested_block(conf_layers(t)) for t in ("high", "mid", "low")}
    # ⚠ **笔宽元数据**（2026-10-25）：量具要判「三档笔重递减」，
    #   而**从 path 的 bbox 反推笔宽是测错的**——笔划近水平，
    #   bbox 高度被**中心线的起伏**主导（起伏 4.2 单位 ≫ 笔宽 3.6），
    #   量出来的是「起伏 + 笔宽」，三档只差 20% ⇒ 那样的判据几乎恒真。
    # ⇒ 笔宽由这里给出（它是**定义值**，不是反推值），量具直接读。
    conf_meta = {
        t: {
            "stroke": round(CONF_BASE_WIDTH * CONF_TIERS[t][0], 2),
            "gap": round(CONF_BASE_GAP * CONF_TIERS[t][1], 2),
        }
        for t in ("high", "mid", "low")
    }

    tw, th = bbox(TICK_LINE)
    rw, rh = bbox(RING_LINE)

    #  累计密度斜坡：从最外带往芯数一遍，注释和实测口径都用它
    #
    #  ⚠ 这里累的是**透过率** t，不是覆盖率，所以递推是 `t *= (1-o)`，
    #    印出来的才是 `1-t`。2026-10-12 之前写的是 `cum = 1 - cum*(1-o)`，
    #    印出 0.22 → 0.83 → 0.38 → 0.73 → 0.53 —— **非单调**，而叠层只可能
    #    越叠越浓，第 3 层不该比第 2 层浅。光看形状就知道是错的。
    #    正确值（旧 NEST）：0.22 → 0.41 → 0.56 → 0.69 → **0.80**。
    t = 1.0
    ramp = []
    for _, o in NEST:
        t = t * (1 - o)
        ramp.append(f"{1 - t:.2f}")
    ramp = " → ".join(ramp)
    nest_scales = " / ".join(f"{s:.2f}" for s, _ in NEST)

    js = f"""/**
 *  红铅笔笔迹几何。**生成物，不要手改。**
 *
 *  来源：`ui/scripts/pencilgen.py`
 *  依据：真实红铅笔照片（2026-10-12 用户提供），轮廓由
 *  「中心线 + 宽度剖面」算出，不是手写的 path。
 *
 *  为什么不用 `stroke-width`：等宽矢量线**没有压力**，一眼读成马克笔，
 *  而照片里每一根都是落笔重、收笔尖的。这三支的收放方向反过一次
 *  （上撇远端最粗、底角最细），是 6× 真像素才暴露的，肉眼看整页图
 *  发现不了——所以轮廓必须由程序算，不能靠手调。
 *
 *  横纵比**也踩过两次**，所以它同样是量出来的，不是手感：
 *    勾 {tw:.0f} × {th:.0f} ≈ **{tw/th:.2f} : 1**
 *    圈 {rw:.0f} × {rh:.0f} ≈ **{rw/rh:.2f} : 1**
 *
 *  第四轮：把参考勾归一到同一外框再并排（`target/probe/ref_vs_gen.png`），
 *  量出来**几何本身**也差着，不是只有表面不对：
 *      笔宽 / 勾高   参考 0.10~0.12    旧版 0.17
 *      勾宽 / 勾高   参考 0.75~0.97    旧版 1.48
 *  旧版给横纵比留的「小尺寸下偏高会显得笨」余量方向就反了：参考的勾
 *  本来就**比看起来更窄**。同时补了落笔处那个很短的挑脚——参考里几乎
 *  每笔都先向左下带出 2~3px 再压下去，缺了它就只是一条折线。
 *
 *  宽度剖面（1 单位 = 1 px @ 1×）：
 *    勾  落笔 3.3 → 底角堆积 2.9 → 收笔尖 0.4
 *    圈  两端 1.5 / 中段 1.15，**三道叠画**（照片里的圈是 2~4 道细线）
 *    叉  两笔各自中间 3.4、两端 0.7
 *
 *  ⚠ 两套柔边机制，不能混用：
 *  1. **嵌套**（`TICK_NESTED`）只给粗笔迹。叠笔偏移**产生不了羽化**——
 *     外层那道的最外缘仍是满 opacity 的硬边，叠出来只是变粗 + 多几级
 *     台阶，6× 真像素下和单层没差别（材质台 P1/P2 实测证实）。
 *     嵌套同一中心线、宽度系数 {nest_scales}，累计密度 {ramp}，
 *     密度从芯往外逐级下降。最内层 ×0.30 而收笔处宽度本来就收到 0.4 →
 *     尖端只有 0.12px，自动淡成看不见，和真实铅笔尖一致。
 *  2. **多道叠画**（`RING_MARK` / `RING_LABEL_MARK`）只给细笔迹。圈的单道
 *     只有 1.15~1.5px，套 ×0.30 等于 0.35px——看不见。**细线没有芯边
 *     之分，它的柔边是叠出来的，不是缩出来的。**
 *
 * ## 置信度短划（2026-10-25）
 *
 * 面板那一格**不再显示百分数**，改用一道**笔迹轻重**表示置信程度。
 * ⇒ 三档（high / mid / low）**中心线完全一样**，只有笔重不同：
 * 「同一道批注，按得重一点还是轻一点」。道数、长度、位置都不参与编码。
 *
 * ⚠ 它是**细笔迹** ⇒ 柔边走**多道叠画**那条路（上面第 2 条），
 * 不是嵌套。三道的间距**小于**道宽 ⇒ 叠起来是一道划痕，不是三条线。
 *
 * ⚠ 三档的墨色**一样浓**（每道 0.34，累计 0.713）：差别全在笔重。
 * 若低档还额外调淡，就会重演「低档淡到等于没有 ⇒ 那一档失效」。
 */

/** 勾 · 嵌套羽化（由外带 → 芯，o 递增）。粗笔迹的柔边全靠它。 */
export const TICK_NESTED = {nested_block(tick_n)};
/** 勾 · 中幅档（→ 约 40 × 28）。同一支笔拉宽压扁，笔重剖面不变。 */
export const TICK_NESTED_MID = {nested_block(tick_mid)};
/** 勾 · 印章档（→ 约 50 × 26）。拉宽压扁之后才盖得住三个字。 */
export const TICK_NESTED_BIG = {nested_block(tick_big)};
/** 勾 · 单层平涂。**只作基线对照**，面板不要用这个。 */
export const TICK_PATH =
  '{tick}';

/** 圈 · 三道叠画。照片里的圈是 2~4 道细线，不是一道粗环。 */
export const RING_MARK = {nested_block(ring_m)};

/** 圈住档位名的横长环 · 两道叠画。独立一个圈只是「这里有一个」，语义偏弱；
 *  **圈住字**才是「这一档被圈中了」。笔比独立圈粗——隔着空隙去包住字，
 *  太轻圈不住。 */
export const RING_LABEL_MARK = {nested_block(ring_l_m)};

/** 圈住档位名的横长环 · 中幅档（→ 约 57 × 22.6）。 */
export const RING_LABEL_MARK_BIG = {nested_block(ring_l_big)};

/** 圈住档位名的横长环 · 印章档（→ 约 74 × 28.6）。
 *  单独一个取景框：根 `<svg>` 的 overflow 是 hidden，画到 60×40 之外
 *  会被静默裁掉，表现为「放大了但还是没变大」。 */
export const RING_LABEL_MARK_HUGE = {nested_block(ring_l_huge)};

/** 叉：两笔独立，各自收锋。 */
export const CROSS_PATHS = [
  '{cross_a}',
  '{cross_b}',
];

/** 置信度短划 · 三档（high / mid / low）。
 *
 *  面板那一格**不显示百分数**，这一道划痕就是置信程度本身。
 *  ⚠ 三档**中心线一样**，只有笔重不同 ⇒ 读法是「按得重一点 / 轻一点」。
 *  ⚠ 墨色三档一样浓：差别全在笔重。低档若还调淡 ⇒ 那一档就废了
 *    （实测过：第一版低档 0.6px，淡到等于没有）。 */
export const CONF_STROKES = {{
  high: {conf_strokes['high']},
  mid: {conf_strokes['mid']},
  low: {conf_strokes['low']},
}};

/** 置信度短划的**笔宽元数据**（取景框单位）。
 *
 *  ⚠ 为什么单独导出来：量具要判「三档笔重递减」，而**从 path 的 bbox
 *  反推笔宽是测错的**——笔划近水平，bbox 高度被**中心线的起伏**主导
 *  （起伏 4.2 单位 ≫ 笔宽 3.6）⇒ 量出来的是「起伏 + 笔宽」，
 *  三档只差 20% ⇒ 那样的判据几乎恒真，等于没有守卫。
 *  ⇒ 笔宽是**定义值**，由生成器给出，量具直接读它。 */
export const CONF_META = {{
  high: {{ stroke: {conf_meta['high']['stroke']}, gap: {conf_meta['high']['gap']} }},
  mid: {{ stroke: {conf_meta['mid']['stroke']}, gap: {conf_meta['mid']['gap']} }},
  low: {{ stroke: {conf_meta['low']['stroke']}, gap: {conf_meta['low']['gap']} }},
}};

/** 共用取景框。1 单位 = 1 px @ 1×。 */
export const MARK_VIEW = {{ w: {VIEW_W}, h: {VIEW_H} }};

/** 放大档专用的取景框。base 放得进 MARK_VIEW，mid / big 放不进。 */
export const RING_VIEW = {{ w: {RING_VIEW['w']}, h: {RING_VIEW['h']} }};
export const TICK_VIEW = {{ w: {TICK_VIEW['w']}, h: {TICK_VIEW['h']} }};

"""
    with open(path, "w", encoding="utf-8", newline="\n") as fh:
        fh.write(js)
    #  ⚠ 打的是**字符数**不是字节数 —— 文件里有中文注释，一个汉字 3 字节，
    #    两者能差出十几个百分点。当字节数报会让人以为产物被截断了。
    print(f"wrote {path} ({len(js)} chars, {len(js.encode('utf-8'))} bytes)")
    print(f"tick bbox {tw:.1f} x {th:.1f} = {tw/th:.2f}:1")
    print(f"ring bbox {rw:.1f} x {rh:.1f} = {rw/rh:.2f}:1")


if __name__ == "__main__":
    import sys
    if len(sys.argv) > 1:
        emit_js(sys.argv[1])
    else:
        print("TICK:", ribbon(TICK_LINE, tick_width))
        print("RING_A:", ribbon(RING_LINE, ring_width))
        print("CROSS_A:", ribbon(CROSS_A, cross_width))