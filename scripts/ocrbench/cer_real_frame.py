"""真帧 CER：xtask ocr 的输出 vs 用户给的答案键。

之前只肉眼看「两边读出同样三句话」，没做逐字对照，
而肉眼看会把「反止」这种漏字当成读对了 —— 字符级编辑距离才算数。

按 y 就近匹配（容差 30px），不做全局对齐：
真实 OCR 的行 y 与答案键有十几像素偏移，且错一个字不改行数。

`--explain` 逐条打印「哪条答案配到了哪一行、编辑几个字、哪几行没配上」，
否则总数看不出编辑的**性质**——「认错一个字」和「两行只差 1px
导致配错行」在合计里都只是一个数字，必须能分开看。
"""

import difflib
import json
import re
import sys
from pathlib import Path

ROOT = Path(r"E:\devSpace\chat-servant-m31f")
GT = ROOT / "target" / "probe" / "ocrbench" / "real-ground-truth.json"
OUT = ROOT / "target" / "probe" / "cer2"

# 成句性块里的行：`  0 | y=  10 h=15 | 打包心`
LINE = re.compile(r"^\s*\d+\s*\|\s*y=\s*(-?\d+)\s*h=\s*(\d+)\s*\|\s?(.*)$")

TOL = 30
# 口径默认与 bench_real_cer.py 的「去空白」一致，好让两边的数字直接可比。
# `--keep-space` 出保留空白的第二版（WinRT 会吞英文单词里的空格）。
KEEP_SPACE = "--keep-space" in sys.argv
EXPLAIN = "--explain" in sys.argv
# `--drop-line V6|蓋`：从识别结果里丢掉含这些片段的行，用来测**某一行的贡献度**。
# 例：多检出一个贴纸字 `V6`，它与 `打包` 只差 1px 会抢走配对，
# 于是丢掉它再跑一遍就能证明「差距是不是全在这一行」——
# 数字若正好回到对照档，说明这一行就是**唯一**差异来源。
DROP: set[str] = set()
if "--drop-line" in sys.argv:
    DROP = {s for s in sys.argv[sys.argv.index("--drop-line") + 1].split("|") if s}
# 配对容差是「尺子」的一个维度，必须能被扫：结论若只在某个容差下成立，
# 那它就是靠挑容差挑出来的，不是靠读数。`--tol N` 就是为扫它存在的。
if "--tol" in sys.argv:
    TOL = int(sys.argv[sys.argv.index("--tol") + 1])
    if TOL < 1:
        raise SystemExit(f"--tol 必须 >= 1，收到 {TOL}")


def norm(s: str, keep_space: bool) -> str:
    """与 bench_real_cer.norm 逐字同一套：全角转半角 / 去空白 / 小写。

    不共用它的实现而重写一遍是故意的——这条脚本要能在**不引入
    bench_real_cer 的模型加载路径**的前提下单独跑；口径一致靠的是
    两边都写着同一段，而不是靠 import。
    """
    s = "".join(chr(ord(c) - 0xFEE0) if 0xFF01 <= ord(c) <= 0xFF5E else c for c in s)
    if not keep_space:
        s = re.sub(r"\s+", "", s)
    return s.lower()


def parse(path):
    rows = []
    for line in path.read_text(encoding="utf-8-sig", errors="replace").splitlines():
        m = LINE.match(line)
        if m:
            rows.append((int(m.group(1)), m.group(3).strip()))
    return rows


def cer(a, b):
    """a=真值 b=识别。返回 (编辑距离, 参考长度)。"""
    if not a:
        return len(b), 0
    prev = list(range(len(b) + 1))
    for i, ca in enumerate(a, 1):
        cur = [i]
        for j, cb in enumerate(b, 1):
            cur.append(min(prev[j] + 1, cur[j - 1] + 1, prev[j - 1] + (ca != cb)))
        prev = cur
    return prev[-1], len(a)


def main():
    gt = json.loads(GT.read_text(encoding="utf-8"))
    frames = {f["name"]: f for f in gt["frames"]}

    # `rapid` 是参照实现 rapidocr，由 dump_rapid_hyps.py 导成同一份格式，
    # 因此走的是**同一段配对代码**——两把尺子量出来的数不可比（见 ADR 0006 第四（四）节）。
    backends = ["tract", "ort", "rapid"]
    grand = {b: [0, 0, 0, 0] for b in backends}  # [edits, refchars, matched, total_truth]

    for name, fr in frames.items():
        truth = fr["truth"]
        print(f"\n=== {name} ===")
        for b in backends:
            p = OUT / f"{name}-{b}.txt"
            if not p.is_file():
                print(f"  {b}: 缺 {p.name}")
                continue
            rows = parse(p)
            if DROP:
                before = len(rows)
                rows = [r for r in rows if not any(d in r[1] for d in DROP)]
                # 量具自证：过滤数必须打印出来。`dropped == 0` 说明片段没匹配上，
                # 此时 CER 若恰好变好，只能说明别的行也变好了——结论作废。
                print(f"  {b:6s} 丢掉 {before - len(rows)}/{before} 行（片段 {'|'.join(sorted(DROP))}）")
            used = set()
            edits = ref = matched = 0
            pairs = []
            for t in truth:
                ty, txt = t["y"], norm(t["text"], KEEP_SPACE)
                best, bestd = None, TOL + 1
                for idx, (oy, ot) in enumerate(rows):
                    if idx in used:
                        continue
                    d = abs(oy - ty)
                    if d < bestd:
                        best, bestd = idx, d
                if best is None:
                    edits += len(txt)          # 整行漏检 = 全部字符错
                    ref += len(txt)
                    pairs.append((ty, t["text"], None, "", len(txt)))
                    continue
                used.add(best)
                d, n = cer(txt, norm(rows[best][1], KEEP_SPACE))
                edits += d
                ref += n
                matched += 1
                pairs.append((ty, t["text"], rows[best][0], rows[best][1], d))
            total = len(truth)
            grand[b][0] += edits
            grand[b][1] += ref
            grand[b][2] += matched
            grand[b][3] += total
            rate = (edits / ref * 100) if ref else 0.0
            print(f"  {b:6s} 命中 {matched}/{total} 行  CER {rate:6.2f}%  "
                  f"(编辑 {edits} / 真值 {ref} 字符)  实读 {len(rows)} 行")
            if not EXPLAIN:
                continue
            for ty, tt, oy, got, d in pairs:
                mark = "OK" if d == 0 else "!!"
                where = "整行漏检" if oy is None else f"y={oy:4d}"
                print(f"       {mark} 真值y={ty:4d} {tt!r} -> {where} {got!r}"
                      f"  编辑 {d}")
            for idx, (oy, ot) in enumerate(rows):
                if idx not in used:
                    print(f"       虚检        y={oy:4d} {ot!r}")

    print("\n=== 合计 ===")
    for b in backends:
        e, r, m, t = grand[b]
        rate = (e / r * 100) if r else 0.0
        print(f"  {b:6s} 命中 {m}/{t} 行  CER {rate:6.2f}%  (编辑 {e} / 真值 {r} 字符)")

    e1, r1, m1, t1 = grand["tract"]
    e2, r2, m2, t2 = grand["ort"]
    if r1 and r2:
        print(f"\n  ort 相对 tract：CER {e2/r2*100:.2f}% vs {e1/r1*100:.2f}%"
              f"（差 {(e2/r2 - e1/r1)*100:+.2f} 个百分点）")
    if r3 := grand.get("rapid"):
        if r3[1]:
            print(f"  参照：rapidocr CER {r3[0]/r3[1]*100:.2f}% vs 本仓 ort {e2/r2*100:.2f}%"
                  f"（差 {(e2/r2 - r3[0]/r3[1])*100:+.2f} 个百分点，"
                  f"同一段配对代码、同一把 ±{TOL}px 尺子）")
    return 0


if __name__ == "__main__":
    sys.exit(main())
