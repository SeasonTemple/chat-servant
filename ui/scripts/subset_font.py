"""Subset LXGW WenKai Lite to a shippable size.

⚠ NOT WIRED UP. Nothing in `ui/` loads these outputs — the `@font-face` that
  pointed at them was removed on 2026-10-14. The body font is FangSong, a font
  already installed on Windows (ticket 22, "霞鹜文楷评估").
  This script is kept as the *reproduction* for the measured sizes below, so
  nobody re-derives them from a blog post. Re-run it before reopening the
  question; do not trust numbers quoted from elsewhere.

Why: the full TTF is 14.9 MB and the Lite build is still 13.3 MB. Neither can
go into an installer. This produces two woff2 subsets so the size/character
coverage trade-off is a measured choice rather than a guess.

Measured 2026-10-14 on this machine:
  LXGW-WenKai-gb2312.woff2   1.54 MB   6763 hanzi
  LXGW-WenKai-cjk.woff2      4.28 MB  20992 hanzi
The "GB2312 subset is ~300 KB" figure floating around the web is off by 5x.

Outputs (next to the input font):
  LXGW-WenKai-gb2312.woff2  only chars encodable in GB2312
  LXGW-WenKai-cjk.woff2    the whole CJK Unified Ideographs basic block

Run: uv run --with fonttools --with brotli python subset_font.py <in.ttf>
"""

import subprocess
import sys
from pathlib import Path

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

BASE_UNICODES = [
    (0x0020, 0x007E),  # Basic Latin
    (0x00A0, 0x00FF),  # Latin-1 Supplement
    (0x2000, 0x206F),  # General Punctuation (dashes, quotes, ellipsis)
    (0x3000, 0x303F),  # CJK Symbols and Punctuation (。、《》)
    (0xFF00, 0xFFEF),  # Halfwidth and Fullwidth Forms
]


def gb2312_charset() -> set[int]:
    """Every codepoint in the CJK block that GB2312 can actually encode."""
    keep = set()
    for cp in range(0x4E00, 0xA000):
        try:
            chr(cp).encode("gb2312")
        except UnicodeEncodeError:
            continue
        keep.add(cp)
    return keep


def ranges_from(codepoints: set[int]) -> str:
    out = []
    start = prev = None
    for cp in sorted(codepoints):
        if start is None:
            start = prev = cp
        elif cp == prev + 1:
            prev = cp
        else:
            out.append((start, prev))
            start = prev = cp
    if start is not None:
        out.append((start, prev))
    # ⚠ Bare hex, no `U+` prefix: fontTools' `--unicodes-file` parser feeds
    #   the text straight to int(x, 16), and `U+4E00` blows up with an empty
    #   literal. Cost: one confusing traceback pointing at fontTools internals.
    return ",".join(f"{a:X}-{b:X}" for a, b in out)


def main() -> None:
    src = Path(sys.argv[1])
    if not src.is_file():
        raise SystemExit(f"input not found: {src}")

    cjk = {cp for lo, hi in BASE_UNICODES for cp in range(lo, hi + 1)}
    gb = gb2312_charset()

    jobs = [
        ("LXGW-WenKai-gb2312.woff2", ranges_from(cjk | gb)),
        ("LXGW-WenKai-cjk.woff2", ranges_from(cjk | set(range(0x4E00, 0xA000)))),
    ]

    for name, unicodes in jobs:
        dst = src.parent / name
        # ⚠ The unicodes list is far too long for a command line on Windows
        #   (WinError 206), so it goes in a file. Also invoke the module rather
        #   than the `pyftsubset` shim so no PATH lookup is involved.
        spec = src.parent / f"{name}.unicodes.txt"
        spec.write_text(unicodes, encoding="ascii")
        cmd = [
            sys.executable,
            "-m",
            "fontTools.subset",
            str(src),
            f"--unicodes-file={spec}",
            "--flavor=woff2",
            "--layout-features=*",
            "--no-hinting",
            "--desubroutinize",
            f"--output-file={dst}",
        ]
        subprocess.run(cmd, check=True)
        spec.unlink()
        print(f"{name}: {dst.stat().st_size / 1024:,.0f} KB")

    print(f"gb2312 covers {len(gb)} hanzi out of 20992 in the CJK basic block")


if __name__ == "__main__":
    main()
