#!/usr/bin/env python3
"""统计 target/debug/deps 里**同一个包名堆了几个哈希版本**。

为什么需要这个量具
------------------
`target/` 膨胀有两种完全不同的成因，处置方式相反：
  ① 单次产物就大 → 调 profile（`debug` / `incremental` / `codegen-units`）；
  ② 单次不大，但**同一个包堆了几十个旧哈希版本** → Cargo 永不删除，
     只能靠 `cargo clean`，调 profile 一点用没有。

`Cargo.toml` 里 `[profile.dev] debug = 1` + `[profile.dev.package."*"] debug = 0`
治的是 ①。而 AGENTS.md 自己记过 ②（"改 feature 会产生新哈希 rlib"）。
⇒ 在动手调 profile 之前，必须先分清是哪种。**这个量具只量 ②。**

怎么读结果
----------
`versions` = 该包名在 deps 下有几个不同哈希。
  · versions ≈ 1–2  → 正常，profile 说了算
  · versions ≫ 2     → 是残留，光调 profile 一点用没有
`rlib+rmeta` 计数相同时（未构建过非 lib 目标）不重复计体积。

⚠ 哈希不是唯一的「多版本」来源：同名不同**扩展名**（rlib/rmeta/dll/exe/pdb）
  属于同一次构建的正常产物，所以按扩展名分开列，不混算成两个版本。

用法：python ui/scripts/targetaudit.py [deps目录]
"""

import os
import re
import sys
from collections import defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
DEPS = Path(sys.argv[1]) if len(sys.argv) > 1 else ROOT / "target" / "debug" / "deps"

# 本脚本输出全是中文，而 Windows 上 Python 的 stdout 默认是传统代码页 cp1252
# —— 不处理就会 UnicodeEncodeError。样板见 scripts/targetgc.py；
# 真正被它咬到的那次是 scripts/fetch_models.py 在 GitHub runner 上崩
# （run 39037571268），同一个仓、同一类错。
for _stream in (sys.stdout, sys.stderr):
    if hasattr(_stream, "reconfigure"):
        _stream.reconfigure(encoding="utf-8", errors="replace")

# libwindows-1a2b3c4d5e6f7890.rlib / windows-capture-abc123.rmeta / app.exe / app.pdb
HASHED = re.compile(r"^(lib)?(.+?)-([0-9a-f]{8,32})$")

by_pkg = defaultdict(lambda: {"hashes": set(), "bytes": 0, "files": 0})

for entry in os.scandir(DEPS):
    if not entry.is_file():
        continue
    name = entry.name
    stem, _, ext = name.rpartition(".")
    m = HASHED.match(stem)
    pkg = m.group(2) if m else stem
    rec = by_pkg[pkg]
    rec["files"] += 1
    try:
        rec["bytes"] += entry.stat().st_size
    except OSError:
        pass
    if m:
        rec["hashes"].add(m.group(3))

rows = sorted(by_pkg.items(), key=lambda kv: -len(kv[1]["hashes"]))
total = sum(r["bytes"] for _, r in rows)

print(f"deps = {DEPS}")
print(f"{len(rows)} distinct package names, {total / 2**30:.2f} GiB\n")
print(f"{'package':<34} {'versions':>8} {'files':>6} {'MiB':>9}")
print("-" * 60)

multi = 0
for pkg, rec in rows[:25]:
    v = len(rec["hashes"])
    if v > 2:
        multi += rec["bytes"]
    flag = "  <== MULTI" if v > 2 else ""
    print(f"{pkg:<34} {v:>8} {rec['files']:>6} {rec['bytes'] / 2**20:>9.1f}{flag}")

if multi:
    print(f"\n=> multi-version packages hold {multi / 2**30:.2f} GiB "
          f"({multi / total:.0%} of deps). Cleaning those is a profile-neutral fix.")
else:
    print("\n=> No package has more than 2 hash versions. The bloat is NOT stale "
          "hashes; profile settings are the lever instead.")