"""生成带精确标准答案的合成 IM 聊天截图语料。

## 为什么必须合成

字符错误率的分母是**人工校对过的标准答案**。拿真实微信帧跑，
没有 ground truth 就只能靠肉眼看——那既不可复现也不可统计。
合成语料不涉及任何真人聊天记录（符合硬约束第 1 条），
而且每一行的答案就是渲染上去的那个字符串，**逐字已知**。

## 复现性

- 网格是固定的：lang × 字号 × 明暗 × seed，全组合，无随机采样。
- 唯一的随机源是 `random.Random(20261017)`，只用来挑句子和抖动内边距。
- 换机器重跑应当得到**逐字节相同**的 PNG（同 Pillow 版本 + 同字体）。

## 刻意不做的事

- **不做气泡尾巴 / 头像 / 时间戳。** 它们会制造与文字无关的检测框，
  污染漏检率的分母。真实界面里有，但不是这轮要测的变量。
- **不换行。** 每个气泡都是单行视觉行，于是「一条 ground truth
  对应一条 OCR 行」成立，行级对齐不用猜。换行会带来多行合并的歧义，
  那是另一个实验。
"""

from __future__ import annotations

import json
import random
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont
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

ROOT = Path(__file__).resolve().parents[2]
OUT = ROOT / "target" / "probe" / "ocrbench"
CORPUS = OUT / "corpus"
TRUTH = OUT / "truth.json"

SEED = 20261017
FONTS = Path(r"C:\Windows\Fonts")

# 语言 → 字体。真微信中文用雅黑、英文走系统西文，两者不共用一个字形集，
# 混排时中文仍由雅黑承担，所以 mixed 也用 msyh。
FONT_BY_LANG = {
    "zh": "msyh.ttc",
    "en": "segoeui.ttf",
    "mixed": "msyh.ttc",
}

SIZES = [12, 14, 16, 19, 21]
THEMES = ["light", "dark"]
LANGS = ["zh", "en", "mixed"]
SEEDS = [0, 1]

# 取自真实 IM 的句式，刻意控制在 4~16 字，保证单行放得下。
POOL = {
    "zh": [
        "明天下午三点开会吗",
        "我这边刚忙完",
        "你说的那个方案我看过了",
        "记得带伞今天有雨",
        "好的我马上到",
        "稍等我查一下日历",
        "这个版本我改完了",
        "地铁上信号不太好",
        "周末去爬山吗",
        "我把会议纪要发群里了",
        "文档链接发你邮箱了",
        "服务器昨晚重启过一次",
    ],
    "en": [
        "Can you send me the file",
        "I will join the call in five minutes",
        "The report is ready for review",
        "Let's move it to Thursday",
        "Did you get my message",
        "I am on the train right now",
        "Please confirm the budget number",
        "Sounds good talk to you later",
        "I pushed the fix to the branch",
        "Can we do a quick sync tomorrow",
    ],
    "mixed": [
        "版本 v2 已经上传了",
        "会议室在 3 楼 302",
        "会议改到 15:30 好吗",
        "我把 link 发到群里了",
        "请确认 Q3 的预算 number",
        "文件 2024Q1 报告.pdf 已发",
        "明天 9:00 见 thanks",
        "记得带 ID 卡和 laptop",
        "接口返回 500 了 retry 一次",
        "文档在 doc 目录第 2 节",
    ],
}

# 两套配色。深色不是把浅色调暗，而是另外选的一组值——
# 对比度要真的像微信深色模式，否则测的是「低对比度」而不是「深色」。
THEME_COLORS = {
    "light": {
        "bg": (237, 237, 237),
        "peer_bubble": (255, 255, 255),
        "peer_text": (25, 25, 25),
        "self_bubble": (149, 236, 105),
        "self_text": (25, 25, 25),
    },
    "dark": {
        "bg": (17, 17, 17),
        "peer_bubble": (44, 44, 44),
        "peer_text": (229, 229, 229),
        "self_bubble": (62, 181, 117),
        "self_text": (255, 255, 255),
    },
}

MARGIN = 24
PEEK = 3  # 纵向留白，避免气泡贴边被 det 截断


def render(lang: str, size: int, theme: str, seed: int) -> dict:
    """渲染一张语料图，返回带 ground truth 的记录。"""
    rng = random.Random(f"{SEED}-{lang}-{size}-{theme}-{seed}")
    font = ImageFont.truetype(str(FONTS / FONT_BY_LANG[lang]), size)
    col = THEME_COLORS[theme]

    # 挑 5 条不重复的句子
    n_lines = 5
    texts = rng.sample(POOL[lang], n_lines)
    # 说话人交替但起点随机，保证两种气泡都出现且左右分布不固定
    first_is_self = rng.random() < 0.5

    probe = ImageDraw.Draw(Image.new("RGB", (8, 8)))
    pad_x = max(6, round(size * 0.6))
    pad_y = max(3, round(size * 0.35))
    gap_y = max(8, round(size * 0.7))

    boxes = []
    for t in texts:
        bx0, by0, bx1, by1 = probe.textbbox((0, 0), t, font=font)
        boxes.append((bx1 - bx0, by1 - by0))

    widest = max(w + 2 * pad_x for w, _ in boxes)
    width = max(520, MARGIN * 2 + widest)
    total_h = PEEK
    for _, h in boxes:
        total_h += h + 2 * pad_y + gap_y
    total_h += PEEK - gap_y
    height = total_h

    img = Image.new("RGB", (width, height), col["bg"])
    draw = ImageDraw.Draw(img)

    lines = []
    y = PEEK
    for i, text in enumerate(texts):
        tw, th = boxes[i]
        speaker = "self" if (i % 2 == 0) == first_is_self else "peer"
        if speaker == "self":
            x = width - MARGIN - tw - 2 * pad_x
            fill = col["self_bubble"]
            ink = col["self_text"]
        else:
            x = MARGIN
            fill = col["peer_bubble"]
            ink = col["peer_text"]

        draw.rounded_rectangle(
            [x, y, x + tw + 2 * pad_x, y + th + 2 * pad_y],
            radius=max(4, round(size * 0.35)),
            fill=fill,
        )
        # 字形实际落点：textbbox 有 bearing，往内缩 pad + 上 bearing
        draw.text((x + pad_x - bx0, y + pad_y - by0), text, font=font, fill=ink)

        lines.append(
            {
                "idx": i,
                "text": text,
                "speaker": speaker,
                # ground truth 的行带：文字本身的高度，不含气泡内边距
                "band": [
                    x + pad_x,
                    y + pad_y,
                    x + pad_x + tw,
                    y + pad_y + th,
                ],
                "bubble": [
                    x,
                    y,
                    x + tw + 2 * pad_x,
                    y + th + 2 * pad_y,
                ],
                "char_count": len(text),
            }
        )
        y += th + 2 * pad_y + gap_y

    ident = f"{lang}_{size}_{theme}_s{seed}"
    path = CORPUS / f"{ident}.png"
    img.save(path)

    return {
        "id": ident,
        "path": str(path.relative_to(ROOT)).replace("\\", "/"),
        "lang": lang,
        "font_px": size,
        "font_file": FONT_BY_LANG[lang],
        "theme": theme,
        "seed": seed,
        "width": width,
        "height": height,
        "lines": lines,
    }


def main() -> None:
    CORPUS.mkdir(parents=True, exist_ok=True)
    images = [
        render(lang, size, theme, seed)
        for lang in LANGS
        for size in SIZES
        for theme in THEMES
        for seed in SEEDS
    ]
    TRUTH.parent.mkdir(parents=True, exist_ok=True)
    TRUTH.write_text(
        json.dumps(
            {
                "generator": "scripts/ocrbench/gen_corpus.py",
                "seed": SEED,
                "normalization": "待评测脚本定义，见 bench_lib.normalize",
                "images": images,
            },
            ensure_ascii=False,
            indent=1,
        ),
        encoding="utf-8",
    )
    n_lines = sum(len(im["lines"]) for im in images)
    print(f"图 {len(images)} 张，ground truth 行 {n_lines} 条 -> {TRUTH}")


if __name__ == "__main__":
    main()
