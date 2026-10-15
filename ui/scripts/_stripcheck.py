"""查 `strip.png` 四条边到底有没有真的撕口 —— 票 44。

## 为什么查这个

用户截图里用红框标了三条**直线**：两条横、一条竖。
而撕口是锯齿状的不规则边缘，直线不可能是撕口。
⇒ **要么撕口没画上（那一侧是全透明），要么别的东西画了直线。**
这是两个不同的病因，必须分开验，不能靠猜。

本脚本对四条边分别报：
  · 带内**完全不透明**的像素数 / 带内**完全不透明**应当存在的比例
  · 每个位置的内缩深度序列（看它是否起伏 = 有撕口，还是齐平 = 是直线）

判据：
  有撕口 ⇒ 深度序列**起伏**（至少 3 个不同值），且带内有相当比例的半透明
  是直线 ⇒ 深度序列**全等**，或整条带全透明

跑：uv run --with pillow python ui/scripts/_stripcheck.py
"""

from __future__ import annotations

from pathlib import Path

from PIL import Image

HERE = Path(__file__).resolve().parent.parent  # ui/
STRIP = HERE / "static" / "strip.png"
OUT = Path("target/probe/stripcheck.txt")

L: list[str] = []
p = L.append


def main() -> None:
    im = Image.open(STRIP).convert("RGBA")
    w, h = im.size
    px = im.load()

    # CSS 那边写的数（PaperPanel.svelte）
    SLICE_CSS = 32
    p(f"strip.png {w}x{h}   CSS slice {SLICE_CSS} 源 px = {SLICE_CSS // 2} CSS px")
    p("")

    # ── 顶边 / 底边：沿 x 看每个位置的第一个不透明 y ────────────────
    def depth_top(x: int) -> int:
        for y in range(SLICE_CSS):
            if px[x, y][3] >= 200:
                return y
        return -1

    def depth_bot(x: int) -> int:
        for y in range(SLICE_CSS):
            if px[x, h - 1 - y][3] >= 200:
                return y
        return -1

    def depth_left(y: int) -> int:
        for x in range(SLICE_CSS):
            if px[x, y][3] >= 200:
                return x
        return -1

    def depth_right(y: int) -> int:
        for x in range(SLICE_CSS):
            if px[w - 1 - x, y][3] >= 200:
                return x
        return -1

    for name, fn in (
        ("顶边", depth_top),
        ("底边", depth_bot),
        ("左边", depth_left),
        ("右边", depth_right),
    ):
        ds = [fn(i) for i in range(SLICE_CSS)]
        uniq = sorted(set(ds))
        opaque = sum(1 for d in ds if d >= 0)
        p(f"── {name}（沿边 {SLICE_CSS} 个位置）")
        p(f"   深度序列: {ds}")
        p(f"   不同值 {len(uniq)} 个: {uniq}")
        p(f"   带内有纸的位置 {opaque}/{SLICE_CSS}")
        if len(uniq) == 1:
            p(f"   ⇒ ❌ **整条边齐平 —— 这是一条直线，不是撕口**")
        elif -1 in uniq:
            p(f"   ⇒ ⚠ 有 {uniq.count(-1)} 个位置**整条带全透明**"
              f"（那部分画出来什么也没有）")
        else:
            p(f"   ⇒ ✅ 起伏 {uniq[0]}~{uniq[-1]} 源 px，有撕口")
        p("")

    # ── 中间纸芯：必须完全不透明且是纸色 ──────────────────────────
    core = [px[x, y] for x in (w // 2 - 4, w // 2, w // 2 + 4)
            for y in (h // 2 - 4, h // 2, h // 2 + 4)]
    p("── 中间纸芯取样")
    p(f"   {core[:6]}")
    p(f"   全不透明: {all(c[3] >= 250 for c in core)}")
    p("")

    # ── 四角：alpha 应当是渐变而不是齐 ────────────────────────────
    p("── 四角 alpha 沿对角线（看撕口的渐变有没有被九宫格截断）")
    for cn, (ox, oy, dx, dy) in {
        "左上": (0, 0, 1, 1),
        "右上": (w - 1, 0, -1, 1),
        "左下": (0, h - 1, 1, -1),
        "右下": (w - 1, h - 1, -1, -1),
    }.items():
        a = [px[ox + dx * k, oy + dy * k][3] for k in range(0, SLICE_CSS, 2)]
        p(f"   {cn}: {a}")
    p("")
    p("⚠ 判据：角上 alpha 应当**从 0 爬到 255**。"
      "若整条都是 0 或整条都是 255，说明这一角裁到的是空白或实心纸。")

    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_text("\n".join(L), encoding="utf-8")
    print(f"written: {OUT}")


if __name__ == "__main__":
    main()
