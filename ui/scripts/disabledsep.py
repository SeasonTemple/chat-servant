#!/usr/bin/env python3
"""算「禁用态怎么表达」在各表达方案下的对比度（票 23 最后一条验收项）。

为什么可以纯算
--------------
这张票卡住的不是「画出来好不好看」，而是**规范（§5.4 `opacity .34`）、
代码（`0.45`）、debug 页（`border-style: dashed`）三方对不上**，
要先定口径。而定口径需要的是**数字**，不是图。

⚠ 票里记着「这个状态现在测不到」——`.cs-btn:disabled` 绑 `busy[p.id]`，
provider 在途时才有，没配 API key 时恒 false，页面上根本不出现禁用按钮。
**但这不妨碍算**：`opacity` 是 CSS 合成运算，结果完全由前景色、背景色、
alpha 三个数决定，与「元素此刻在不在页面上」无关。
⇒ 于是绕开「造一个禁用态」这个死结，直接把三组数算出来。

算的是什么
----------
`opacity` 作用于**整个元素**——文字、边框、底色一起被稀释，而不只是文字。
合成式：`out = fg * a + bg * (1 - a)`，其中 `bg` 是元素所在页面底色。
对每个按钮算三个数：
  ① 文字 vs 自身底   —— 「还读得出字吗」
  ② 边框 vs 页底     —— 「还看得出这个控件吗」（描边型才有）
  ③ 自身底 vs 页底   —— 「墨底药丸还在吗」（墨底型才有）

⚠ **先说清判据的边界**：WCAG 1.4.3 / 1.4.11 对 `disabled` 控件是**明确豁免**的
（disabled 元素不要求满足对比度）。所以这里算的**不是合规判据**，
而是「用户还能不能看出这里有个按钮、知道它不能点」这一条**设计可读性**。
把它当硬门槛会逼出一个没人真的需要的数字；当它不存在又会让三种方案看起来一样好。
⇒ 输出同时给两个数：相对**正常态**的损失，以及绝对值。

用法：python ui/scripts/disabledsep.py
"""

import sys

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

# ── 取值来自 ui/src/app.css 与 PanelSettings.svelte（逐条 grep 过）──
PAGE = (237, 240, 241)        # --cs-paper        设置页底 rgb(237 240 241)
INK = (20, 26, 31)           # --cs-ink          文字/墨底 rgb(20 26 31)
PAPER_RAISED = (248, 250, 250)  # --cs-paper-raised 墨底上的字 rgb(248 250 250)
RULE_STRONG = (185, 195, 200)   # --cs-rule-strong  .cs-btn-mini 描边
LIVE = (200, 50, 42)          # --cs-live         .cs-btn-mini.primary 描边


def lum(rgb):
    out = []
    for c in rgb[:3]:
        s = c / 255.0
        out.append(s / 12.92 if s <= 0.04045 else ((s + 0.055) / 1.055) ** 2.4)
    r, g, b = out
    return 0.2126 * r + 0.7152 * g + 0.0722 * b


def ratio(a, b):
    la, lb = lum(a), lum(b)
    hi, lo = max(la, lb), min(la, lb)
    return (hi + 0.05) / (lo + 0.05)


def over(fg, bg, alpha):
    """CSS opacity: 把 fg 按 alpha 稀释到 bg 上。alpha=0 时完全看不见。"""
    return tuple(fg[i] * alpha + bg[i] * (1 - alpha) for i in range(3))


def report(name, variants):
    print(f"\n{name}")
    print(f"  {'variant':<22}{'text/fill':>12}{'rule/page':>13}{'fill/page':>11}")
    print("  " + "-" * 60)
    for label, text_r, rule_r, fill_r in variants:
        f_text = f"{ratio(text_r[0], text_r[1]):>6.2f}:1"
        f_rule = f"{ratio(rule_r[0], rule_r[1]):>7.2f}:1" if rule_r else " " * 9 + "-"
        f_fill = f"{ratio(fill_r[0], fill_r[1]):>5.2f}:1" if fill_r else " " * 6 + "-"
        print(f"  {label:<22}{f_text:>12}{f_rule:>13}{f_fill:>11}")


def main():
    print("Disabled-state contrast, computed (no browser, no page render)")
    print(f"page={PAGE}  ink={INK}  rule-strong={RULE_STRONG}")
    print()
    print("NOTE: WCAG 1.4.3 / 1.4.11 EXEMPT disabled controls from contrast")
    print("      requirements. These numbers are a *design legibility* yardstick")
    print("      ('can the user still see the control and that it is inert'),")
    print("      NOT a compliance gate.")

    # ── .cs-btn-ink：墨底药丸，白字（唯一实心）──
    ink_variants = [
        ("enabled (baseline)", (PAPER_RAISED, INK), None, (INK, PAGE)),
        ("opacity: .34", (over(PAPER_RAISED, PAGE, .34), over(INK, PAGE, .34)),
         None, (over(INK, PAGE, .34), PAGE)),
        ("opacity: 0.45", (over(PAPER_RAISED, PAGE, .45), over(INK, PAGE, .45)),
         None, (over(INK, PAGE, .45), PAGE)),
        ("border dashed (no dim)", (PAPER_RAISED, INK), None, (INK, PAGE)),
    ]
    report(".cs-btn-ink  (solid ink pill, white text)", ink_variants)

    # ── .cs-btn-mini：透明底 + 1px 描边，墨字 ──
    mini_variants = [
        ("enabled (baseline)", (INK, PAGE), (RULE_STRONG, PAGE), None),
        ("opacity: .34", (over(INK, PAGE, .34), PAGE),
         (over(RULE_STRONG, PAGE, .34), PAGE), None),
        ("opacity: 0.45", (over(INK, PAGE, .45), PAGE),
         (over(RULE_STRONG, PAGE, .45), PAGE), None),
        ("border dashed (no dim)", (INK, PAGE), (RULE_STRONG, PAGE), None),
    ]
    report(".cs-btn-mini  (transparent, 1px rule, ink text)", mini_variants)

    print()
    print("Reading:")
    print("  * opacity dims BORDER and FILL too, not just text - that is why")
    print("    debug/+page.svelte rejected it and the spec still says .34.")
    print("  * Compare each row's 'text vs fill' against its OWN baseline, not")
    print("    against 4.5:1 - the question is 'can it still be read', not")
    print("    'does it pass AA' (disabled is exempt either way).")
    return 0


if __name__ == "__main__":
    sys.exit(main())