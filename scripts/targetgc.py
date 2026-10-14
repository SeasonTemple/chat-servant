#!/usr/bin/env python3
"""target/ 回收 —— Cargo 没有内置 GC，这一段机制补上（票 34 附带产出）。

为什么需要它
------------
**Rust 1.88（2025-06）确实加了 Cargo 自动 GC，但它不管 target/。**
官方原文是 "garbage collection on the cache in its home directory" ——
即 `~/.cargo` 里下载的 `.crate`、registry 索引、git 检出；
而 `target/` 里的编译产物、增量缓存、旧哈希 rlib **一个都不管**。
（本机 cargo 1.94.1 的 `cargo --list` 里也确实只有 `clean`，没有 `gc`——
 `gc` 子命令在 cargo#13060 里仍是 unstable。）

⇒ 机制上没有自动方案，只能自己补。这一段就是那个补丁。

安全边界（为什么删了不会编译出坏东西）
----------------------------------------
Cargo 判断「要不要重编」靠 `target/<profile>/.fingerprint`，不靠文件是否存在。
把 `.fingerprint` **留下**，删掉对应的旧产物，Cargo 会发现产物缺失并**重编那一个包**——
结果永远正确，只是那一步多花时间。
⇒ 所以本脚本**绝不删 `.fingerprint`**，只回收它已经「判定为无用」的边角料：
    · `incremental/`  —— 纯加速缓存，删了只影响下次编译快慢
    · `deps/` 里超过 N 天没被动过的旧哈希产物
且按 **mtime** 而不是「哈希看起来旧」来判断：Cargo 每次构建都会 touch
它要用的那些文件，所以「几天没被动过」就等于「这段时间没人用到」。

⚠ 删不掉是正常的：Windows 上正在被链接器/杀软占用的文件会拒绝删除。
  脚本对每个失败**静默跳过并计数**，绝不因为一个锁住的文件就中断或报错。

用法：
    python scripts/targetgc.py                  # 超过阈值就按 7 天清理
    python scripts/targetgc.py --dry-run        # 只报告，不动手
    python scripts/targetgc.py --threshold-gb 6 --age-days 3
"""

import argparse
import os
import shutil
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]

# 只碰这两处。刻意**不含** .fingerprint / build/：前者是 Cargo 的真相来源，
# 后者是构建脚本产物，删了会让依赖被整棵重编，得不偿失。
SWEEPABLE = ("incremental", "deps")


def dir_size(path):
    total = 0
    for dirpath, _dirs, files in os.walk(path):
        for f in files:
            try:
                total += os.stat(os.path.join(dirpath, f)).st_size
            except OSError:
                pass
    return total


def human(n):
    for unit in ("B", "KiB", "MiB", "GiB"):
        if n < 1024 or unit == "GiB":
            return f"{n:.1f} {unit}"
        n /= 1024


def sweep(base, age_days, dry_run):
    cutoff = time.time() - age_days * 86400
    removed = 0
    skipped = 0
    for name in SWEEPABLE:
        d = base / name
        if not d.is_dir():
            continue
        for entry in os.scandir(d):
            try:
                mtime = entry.stat().st_mtime
            except OSError:
                continue
            if mtime >= cutoff:
                continue
            # 整个子目录按 mtime 删（incremental 就是一堆 per-crate 目录），
            # 单文件则逐个删（deps 下是散落的 rlib/rmeta/pdb）。
            victims = []
            if entry.is_dir():
                victims = [os.path.join(dp, f)
                           for dp, _d, fs in os.walk(entry.path) for f in fs]
            else:
                victims = [entry.path]
            for v in victims:
                if dry_run:
                    removed += 1
                    continue
                try:
                    os.remove(v)
                    removed += 1
                except OSError:
                    skipped += 1
            if not dry_run and entry.is_dir():
                shutil.rmtree(entry.path, ignore_errors=True)
    return removed, skipped


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--target", default=str(ROOT / "target"))
    ap.add_argument("--threshold-gb", type=float, default=8.0,
                    help="低于此体积不动手（默认 8）")
    ap.add_argument("--age-days", type=int, default=7,
                    help="只回收超过多少天没被动过的产物（默认 7）")
    ap.add_argument("--dry-run", action="store_true")
    a = ap.parse_args()

    base = Path(a.target)
    if not base.is_dir():
        print(f">>> target not found: {base} (nothing to do)")
        return 0

    before = dir_size(base)
    print(f"target = {human(before)}  ({before / 2**30:.2f} GiB)")

    if before <= a.threshold_gb * 2**30:
        print(f">>> under threshold ({a.threshold_gb} GiB); nothing swept")
        return 0

    removed, skipped = sweep(base / "debug", a.age_days, a.dry_run)
    freed = 0
    after = dir_size(base)
    freed = before - after
    verb = "would free" if a.dry_run else "freed"
    print(f">>> swept {removed} file(s) older than {a.age_days} day(s) "
          f"(skipped {skipped} locked/inaccessible); {verb} {human(freed)}")
    print(f">>> target now {human(after)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())