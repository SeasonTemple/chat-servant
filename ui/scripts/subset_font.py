"""Subset LXGW WenKai Lite to a shippable size.

Why: the full TTF is 13.3 MB and the Lite build is still 13.3 MB. Neither can
go into an installer. This produces two woff2 subsets so the size/character
coverage trade-off is a measured choice rather than a guess.

Outputs (next to the input font):
  LXGW-WenKai-gb2312.woff2  only chars encodable in GB2312
  LXGW-WenKai-cjk.woff2    the whole CJK Unified Ideographs basic block

Run: uv run --with fonttools --with brotli python subset_font.py <in.ttf>
"""

import subprocess
import sys
from pathlib import Path

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
