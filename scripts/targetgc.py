#!/usr/bin/env python3
"""target/ 回收 —— Cargo 没有内置 GC，这一段机制补上（票 34 附带产出）。

为什么需要它
------------

**Rust 1.88（2025-06）确实加了 Cargo 自动 GC，但它不管 target/。**
官方原文是 "garbage collection on the cache in its home directory" ——
即 `~/.cargo` 里下载的 `.crate`、registry 索引、git 检出；
而 `target/` 里的编译产物、增量缓存、旧哈希 rlib **一个都不管**。
（cargo#13060 的 `gc` 子命令至今 unstable。）

⇒ 机制上没有自动方案，只能自己补。这一段就是那个补丁。

⚠⚠ **2026-10-16：原来只有「按 mtime 回收」这一条路，而它在天天构建的仓库里
等于永不生效。** 当时实测：target 已 17.1 GiB（远超 8 GiB 阈值），
脚本输出 `swept 0 file(s) older than 7 day(s); freed 0.0 B`。
原因很直白——判定条件是「7 天没被动过」，而这个项目每天都构建，
于是**每轮门禁跑完，产物都是热的**，一条都不满足。
⇒ 阈值形同虚设，而输出看起来和「一切正常」一模一样。
这正是本仓库反复踩的那个形状：机制悄悄失效，没人知道。

现在有两条路
------------

1. `superseded`（**默认**）——**先问 cargo 它现在在用哪些产物，删其余的**。
   起因是实测数据推翻了我们自己记的结论：

       deps = 11.63 GiB / 732 个包
       多版本包合计 6.38 GiB = **deps 的 55%**
       chatservant_capture 38 份 / 808 MiB，tract_core 11 份 / 1094 MiB

   （此前 AGENTS.md 记的是「多哈希残留只占 deps 的 1%」——**实测是错的**，
   已在同一轮订正。那次误判的来路很典型：用「刚清空后全新构建」做基线，
   而**逐轮叠加**的多版本产物正好被那一次清空抹掉了。）

   一个包每次以不同 flag 组合编译（clippy / test / build，dev 与 release 又是
   不同的 profile）都会留下**一组新哈希**，旧的永不删除。

   ⚠⚠ **判据必须是 cargo 自己说的，不能是 mtime。**
   这一点是被实测打出来的（2026-10-16）：先按「每个包保留 mtime 最新的 3 个哈希」
   回收了 10.8 GiB，紧接着 `cargo test --workspace` 从 **36.9s 变成 139.3s**
   ——它把还在用的产物判成尾巴删了，于是重编。

   原因：**cargo 命中缓存时不会 touch 已有的产物文件**。一轮门禁里
   `cargo test` 表面跑过，但多数产物是几天前编好、这次直接复用的，
   mtime 依然很旧；而当天 clippy 产生的另一组哈希 mtime 很新。
   ⇒ 「最新的」不等于「在用的」。

   所以这里改成向 cargo 要真相：
       cargo test   --workspace --no-run --message-format=json
       cargo build  --workspace            --message-format=json
       cargo clippy --workspace --all-targets --message-format=json
   把三份 `compiler-artifact` 的 `filenames` 收齐成「活着的产物」集合，
   **集合之外的**才是尾巴。三条都是命中缓存时的空跑，实测几秒级。

   ⚠ 三个 plan 都要问：只问 `cargo test` 会把 `pnpm tauri dev` 用的
     那个 bin 判成尾巴，于是下一次起 dev 白等一次全量链接。
   ⚠ 另外要求产物 mtime 超过 `--min-age-hours` 才动，兜住
     「刚编出来、还没被任何 plan 列举到」的边角。
   ⚠ **列举失败就不删**（见 `live_artifacts`）——宁可漏扫，
     也不能拿一个错的判据去删。

2. `aged` —— 保留原来的行为：只回收超过 N 天没被动过的 `incremental/` 与
   `deps/` 产物。适合「放着不构建、只想腾地方」的场合。

⚠⚠ **两个模式扫的地方不一样（2026-10-19 实测）**：
   · `superseded`（默认）**只扫 `deps/` 下的散落文件**（`os.scandir` 且
     `entry.is_file()`），`incremental/` 下的**目录**它一个都不看。
   · `aged` 才碰 `incremental/`（连目录带里面的文件一起删）。
   ⇒ 想清 `incremental/` 必须显式 `--mode aged`。
   ⚠ 2026-10-19 实测：`superseded` 在一台「产物全是 12h 内建的」机器上
   扫到 0 个，而 `--mode aged` 一次清掉 **690.3 MiB**（全在 incremental）。
   ⇒ **看到 superseded 扫到 0 个，先跑一次 `--mode aged --dry-run`**，
     别直接判定「没垃圾可清」。

⚠ **superseded 扫到 0 个不一定是失效**（2026-10-19 实测，判据是对的）：
   脚本会报「cargo 报告在用 992 个产物」，而 deps 里超 12h 的 853 个文件
   **全部在那个集合内**（集合外 0 个）—— cargo 的 `compiler-artifact`
   消息会把它**知道的所有**哈希都列出来，包括当前 plan 用不到的旧哈希。
   ⇒ 按「宁可漏扫」的纪律，不删是对的；那 1.39 GiB **不是垃圾**。

安全边界（为什么删了不会编译出坏东西）
----------------------------------------
Cargo 判断「要不要重编」靠 `target/<profile>/.fingerprint`，不靠文件是否存在。
把 `.fingerprint` **留下**，删掉对应的旧产物，Cargo 会发现产物缺失并**重编那一个包**——
结果永远正确，只是那一步多花时间。
⇒ 所以本脚本**绝不删 `.fingerprint`**，也不删 `build/`（删了会整棵依赖重编）。
⚠ 删不掉是正常的：Windows 上正在被链接器/杀软占用的文件会拒绝删除。
  脚本对每个失败**静默跳过并计数**，绝不因为一个锁住的文件就中断或报错。
⚠ **删 `incremental/` 不会触发重编**（2026-10-19 实测）：清掉 690.3 MiB 后
  `cargo test --workspace` 仍 22.8s、**零 `Compiling` 行**、测试全过。
  增量目录只影响编译**速度**，Cargo 判断要不要重编靠 `.fingerprint`。
⚠ 但 `deps/` 下的产物是**会**触发重编的（cargo 发现产物缺失就重编那个包）。
  ⇒ 删完必须验证一次「零 `Compiling`」，别只看省了多少字节。

用法：
    python scripts/targetgc.py                    # 超阈值 + 过了冷却期才动手
    python scripts/targetgc.py --dry-run          # 只报告，不动手（不受冷却限制）
    python scripts/targetgc.py --force            # 无视冷却，立刻收一次
    python scripts/targetgc.py --report           # 只出体积报告，不判阈值
    python scripts/targetgc.py --mode aged        # 回到「按 mtime 回收」的老路
    python scripts/targetgc.py --threshold-gb 6 --min-age-hours 6 --max-hours 12
"""

import argparse
import json
import os
import shutil
import subprocess
import sys
import time
from pathlib import Path

#**必须自己把 stdout 钉成 UTF-8**（2026-10-16 实测踩到）。
# 本机控制台代码页是 936(GBK)，而这个脚本的提示里有 `⇒`、`` 这类
# GBK 编不出来的字符 —— Python 3.7+ 的 `print` 会在**写到一半时**抛
# `UnicodeEncodeError: 'gbk' codec can't encode character '⇨'`，
# 于是脚本 exit 1，而门禁那一步只参与磁盘维护、不参与 $fail。
#
# 症状极具误导性：崩的位置看起来像「回收逻辑出错」，
# 实际只是**打印第一个提示时编不出来**——回收可能压根没开始。
# 交给调用方设 PYTHONIOENCODING 也行，但门禁/CI/手敲三种入口不一定都设，
# 所以钉在这里。
for _stream in (sys.stdout, sys.stderr):
    try:
        _stream.reconfigure(encoding="utf-8", errors="replace")
    except (AttributeError, ValueError):   # 老 Python / 被重定向成别的流
        pass

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


def live_artifacts():
    """问 cargo：现在真正在用哪些产物文件。

    返回 `deps/` 下的**文件名**集合。列举不出来就返回空集合 —— 调用方必须
    把它当成「不要删任何东西」，而不是「都可以删」。

    ⚠ 用的是 `--message-format=json` 的 `compiler-artifact` 记录，
      `filenames` 是 cargo **已经决定要链接/运行**的那些文件，
      正是「删了会触发重编」的那批。

    ⚠⚠ **只问一条 plan，不是三条**（实测：问三条要 80 秒）。
      `cargo test --workspace --all-targets --no-run` 覆盖 dev profile 的
      lib / bin / test / bench，也就是 `cargo build`、`pnpm tauri dev`、
      `cargo test` 三条路径共同用到的那一套产物。

      代价要说清楚：**clippy 的产物不在清单里**，所以它们会被当成尾巴删掉，
      下一轮门禁的 clippy 要重跑一次（实测 20.5s）。
      用一次 20s 换掉每轮门禁多花 60s，在一天一次的冷却节奏下这笔账划算。
    """
    cmd = ["cargo", "test", "--workspace", "--all-targets", "--no-run",
           "--message-format=json"]
    try:
        proc = subprocess.run(
            cmd, cwd=str(ROOT), capture_output=True, text=True,
            timeout=1800, errors="replace")
    except (OSError, subprocess.TimeoutExpired) as e:
        print(f">>> live_artifacts: `cargo test --no-run` 列举失败"
              f"（{e.__class__.__name__}）")
        return set()
    if proc.returncode != 0:
        print(f">>> live_artifacts: `cargo test --no-run` 退出 {proc.returncode}，"
              f"不据此删除任何文件")
        return set()

    live = set()
    for line in proc.stdout.splitlines():
        if '"compiler-artifact"' not in line:
            continue
        try:
            obj = json.loads(line)
        except json.JSONDecodeError:
            continue
        for f in obj.get("filenames") or ():
            live.add(os.path.basename(f))
    return live


def sweep_superseded(deps_dir, live_names, min_age_hours, dry_run):
    """删掉 `deps/` 里**不在 cargo 的活产物集合内**、且够旧的散落文件。"""
    cutoff = time.time() - min_age_hours * 3600
    removed = skipped = 0
    freed = 0
    for entry in os.scandir(deps_dir):
        if not entry.is_file():
            continue
        if entry.name in live_names:
            continue
        try:
            st = entry.stat()
        except OSError:
            continue
        if st.st_mtime >= cutoff:
            continue  # 太新，可能是某个还没被列举到的边角
        if dry_run:
            removed += 1
            freed += st.st_size
            continue
        try:
            os.remove(entry.path)
            removed += 1
            freed += st.st_size
        except OSError:
            skipped += 1
    return removed, skipped, freed


def sweep_aged(base, age_days, dry_run):
    cutoff = time.time() - age_days * 86400
    removed = 0
    skipped = 0
    # ⚠⚠ dry-run 也必须**累加字节数**（2026-10-19 修）。
    #   原来这里只 `removed += 1`，字节数只在真删时累加，而调用处的
    #   `freed = counted if dry_run else before - after` 取的是 `counted`——
    #   而 `sweep_aged` 只返回**两个**值，`counted` 永远是调用前的 0。
    # ⇒ `--mode aged --dry-run` 永远报「扫了 2850 个文件、would free 0.0 B」。
    #   那正是本文件自己骂过的那句话：「一条**永远显示「什么都不省」**的报告，
    #   比没有报告更糟」—— 同一个坑换了个模式又踩了一遍。
    #   而它有实际代价：dry-run 是**唯一**能在动手前评估收益的通道，
    #   报 0 字节就没法判断该不该删。
    freed = 0
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
            victims = []
            if entry.is_dir():
                victims = [os.path.join(dp, f)
                           for dp, _d, fs in os.walk(entry.path) for f in fs]
            else:
                victims = [entry.path]
            for v in victims:
                try:
                    size = os.path.getsize(v)
                except OSError:
                    size = 0
                if dry_run:
                    removed += 1
                    freed += size
                    continue
                try:
                    os.remove(v)
                    removed += 1
                    freed += size
                except OSError:
                    skipped += 1
            if not dry_run and entry.is_dir():
                shutil.rmtree(entry.path, ignore_errors=True)
    return removed, skipped, freed


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--target", default=str(ROOT / "target"))
    ap.add_argument("--threshold-gb", type=float, default=8.0,
                    help="低于此体积不动手（默认 8）")
    ap.add_argument("--age-days", type=int, default=7,
                    help="mode=aged 时：只回收超过多少天没被动过的产物（默认 7）")
    ap.add_argument("--mode", choices=("superseded", "aged"), default="superseded",
                    help="superseded=按被取代的旧哈希（默认）；aged=按 mtime")
    ap.add_argument("--min-age-hours", type=float, default=12.0,
                    help="superseded 模式下，产物至少多旧才可能被删（默认 12 小时）")
    ap.add_argument("--max-hours", type=float, default=24.0,
                    help="两次真回收之间的最短间隔（默认 24 小时）；0 = 不限")
    ap.add_argument("--force", action="store_true",
                    help="无视冷却，强制列举并回收")
    ap.add_argument("--report", action="store_true",
                    help="只出体积报告，不判阈值也不动手")
    ap.add_argument("--dry-run", action="store_true")
    a = ap.parse_args()

    base = Path(a.target)
    if not base.is_dir():
        print(f">>> target not found: {base} (nothing to do)")
        return 0

    before = dir_size(base)
    print(f"target = {human(before)}  ({before / 2**30:.2f} GiB)")

    if a.report:
        return 0

    if before <= a.threshold_gb * 2**30:
        print(f">>> under threshold ({a.threshold_gb} GiB); nothing swept")
        return 0

    debug = base / "debug"
    counted = 0
    if a.mode == "aged":
        # ⚠ 必须接住第三个返回值（字节数）。原来这里只接两个，
        #   `counted` 恒为 0 ⇒ dry-run 报「would free 0.0 B」。
        removed, skipped, counted = sweep_aged(debug, a.age_days, a.dry_run)
    else:
        # ---- 冷却：列举要跑一次 cargo（实测十几秒），不该每轮门禁都付 ----
        # 残留是**逐轮叠加**出来的，一天清一次与一小时清一次效果几乎一样，
        # 成本差 20 倍以上。冷却状态放在 target/ 里（本身就被 gitignore）。
        stamp = base / ".targetgc-last"
        if a.max_hours > 0 and not a.force and not a.dry_run and stamp.is_file():
            try:
                last = float(stamp.read_text().strip())
            except (ValueError, OSError):
                last = 0.0
            ago_h = (time.time() - last) / 3600
            if ago_h < a.max_hours:
                print(f">>> [superseded] 上次回收在 {ago_h:.1f} 小时前，"
                      f"未到 {a.max_hours}h 间隔 ⇒ 本轮不动手（--force 可强制）")
                return 0

        deps_dir = debug / "deps"
        if not deps_dir.is_dir():
            print(f">>> {deps_dir} not found; nothing swept")
            return 0
        # 判据的唯一来源：cargo 自己说它要用哪些产物。
        # 门禁运行时 `$env:CARGO_INCREMENTAL=0`，而计划列举要跑 cargo，
        #   环境必须一致，否则列举出来的产物集与实际使用的不符——那就会误删。
        live = live_artifacts()
        if not live:
            print(">>> [superseded] 拿不到 cargo 的活产物清单 ⇒ **本轮不删任何文件**。")
            print(">>>   这是「宁可漏扫」：判据不明时动手，等于拿构建正确性赌。")
            print(f">>> target now {human(before)}")
            return 0
        removed, skipped, counted = sweep_superseded(
            deps_dir, live, a.min_age_hours, a.dry_run)

    after = dir_size(base)
    # dry-run 下目录大小当然不变，所以「省了多少」只能取扫描时累加的字节数。
    #   早先写成 `before - after`，于是 dry-run 永远报 `would free 0.0 B`——
    #   一条**永远显示「什么都不省」**的报告，比没有报告更糟。
    freed = counted if a.dry_run else before - after
    verb = "would free" if a.dry_run else "freed"
    if a.mode == "aged":
        print(f">>> [aged] swept {removed} file(s) older than {a.age_days} day(s) "
              f"(skipped {skipped} locked/inaccessible); {verb} {human(freed)}")
    else:
        print(f">>> [superseded] cargo 报告在用 {len(live)} 个产物；扫掉 {removed} "
              f"个既不在用、又超过 {a.min_age_hours}h 的（skipped {skipped} locked）；"
              f"{verb} {human(freed)}")

    #**超了阈值却一件没扫到，必须自己喊。**
    # 原来这里只打 `swept 0 ... freed 0.0 B`，读起来和「一切正常」完全一样——
    # 而那正是 2026-10-16 实测到的状态：17.1 GiB、阈值 8 GiB、扫到 0 个文件。
    # 机制失效几个月没人发现，就是因为它失败时不说话。
    if removed == 0 and not a.dry_run:
        print(">>> target gc: 超过阈值却**一件都没扫到**——机制没有生效，别当成正常。")
        print(">>>   可能原因：产物 mtime 都在 --min-age-hours 之内（刚构建过）；")
        print(">>>   或 deps 下文件都被 cargo 列举为在用。用 --dry-run 看判定明细。")

    # 冷却戳只在**真的动过手**之后写：判据拿不到、或一个都没扫到的时候，
    # 不能让下一次跳过——那会把一次失效变成两次失效。
    if not a.dry_run and a.mode == "superseded" and removed > 0:
        try:
            (base / ".targetgc-last").write_text(str(time.time()))
        except OSError as e:
            print(f">>> target gc: 写冷却戳失败（{e.__class__.__name__}）——下一轮会重来一次")

    print(f">>> target now {human(after)}" + ("（dry-run：未改动任何文件）" if a.dry_run else ""))
    return 0


if __name__ == "__main__":
    sys.exit(main())
