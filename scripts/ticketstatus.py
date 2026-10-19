"""机器点一遍票据，生成 `.scratch/wechat-reply-copilot/STATUS.md`。

## 为什么要有这个脚本（2026-10-15）

问「推送里有现状说明吗」时答不上来——仓库里**没有一份**统一说明「还剩什么」。
现状散在每张票手写的 `**Status:**` 行、`docs/HANDOFF.md`（冷启动包，不是进展）、
`docs/前端设计进度.md`（只管前端）三处。

⚠ 而手写的汇总**必然漂**：2026-10-14/15 一晚之间，票 22 / 23 / 35 的
Status 行连续三次与自己验收项矛盾（「真机未验」而验收项下就贴着实测数据、
`5/10` 实为 `8/8`、「剩 2 条」实为 3 条）。

⇒ 所以这份 STATUS 是**生成的**，不是写的。改完票就重跑：

```bash
python scripts/ticketstatus.py
python scripts/ticketstatus.py --check   # 只报漂移，不写文件
```

⚠ **勾数 ≠ 进度**。很多票的 Status 写着 done 却仍有 1~2 项未勾，
那些是**条件项或永久性说明**（例如夹具 gitignored、缺件即跳过）。
所以脚本同时打两个口径，并在文件里写明这个陷阱。
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
ISSUES = ROOT / ".scratch" / "wechat-reply-copilot" / "issues"
OUT = ROOT / ".scratch" / "wechat-reply-copilot" / "STATUS.md"

# 本脚本输出全是中文，而 Windows 上 Python 的 stdout 默认是传统代码页 cp1252
# —— 不处理就会 UnicodeEncodeError。样板见 scripts/targetgc.py；
# 真正被它咬到的那次是 scripts/fetch_models.py 在 GitHub runner 上崩
# （run 39037571268），同一个仓、同一类错。
for _stream in (sys.stdout, sys.stderr):
    if hasattr(_stream, "reconfigure"):
        _stream.reconfigure(encoding="utf-8", errors="replace")

DONE = ("done", "已修", "已定位", "销账", "验完")
# 分类的**权威判据是 Status 行 `·` 之前那一段（triage 标签）**，不是散文里的词。
# 下面是**前缀**匹配（不是等值）：不少票的标签后面直接跟括号说明，
# 例如 `done（2026-10-07 实机复核…）`，等值匹配会全部落空。
OPEN_LABELS = (
    "needs-triage",
    "needs-info",
    "ready-for-agent",
    "ready-for-human",
    "in-progress",
    "进行中",
)
CLOSED_LABELS = ("done", "部分 done", "wontfix", "销账")

BOX_X = re.compile(r"(?m)^\s*-\s*\[x\]")
BOX_O = re.compile(r"(?m)^\s*-\s*\[ \]")
STATUS = re.compile(r"(?m)^\*\*Status:\*\*\s*(.+)$")
# Status 行里手写的「N/M」。它自己也会漂（票 25 写 5/5，机器点出来 4/4），
# 所以脚本要把它抓出来，而不是照抄。
STATUS_NUM = re.compile(r"(\d+)\s*/\s*(\d+)")
BLOCKED = re.compile(r"(?m)^\*\*Blocked by:\*\*\s*(.+)$")


def collect() -> list[dict[str, str]]:
    rows = []
    for path in sorted(ISSUES.glob("*.md")):
        text = path.read_text(encoding="utf-8")
        st = STATUS.search(text)
        status = st.group(1).strip() if st else "(没写 Status)"
        done = str(len(BOX_X.findall(text)))
        open_ = str(len(BOX_O.findall(text)))
        total = int(done) + int(open_)
        drift = ""
        m = STATUS_NUM.search(status)
        if m and (int(m.group(1)), int(m.group(2))) != (int(done), total):
            # ⚠⚠ **带上原文上下文**（2026-10-19 加）。
            #
            # 这条判据是「Status 行里**第一个** N/M 应当是验收项的勾数」——
            # 而 Status 行是散文，里面**别处的** N/M 会撞上来。
            # 撞上来的实例：票 56 的 Status 写「新增守卫 + 自证 4/4」，
            # 那个 4/4 说的是**自证组数**，不是验收项数（实际 11/11）。
            # ⇒ 光看「Status 写 4/4，实点 11/11」会让人以为票面少勾了七项，
            #   而真相是**量具读错了那个数**。
            # ⇒ 把命中的那一句原文带上，读的人自己就能分辨是「票面写错」
            #   还是「这个 N/M 本来就不是验收项数」。
            lo = max(0, m.start() - 24)
            hi = min(len(status), m.end() + 16)
            ctx = status[lo:hi].replace("\n", " ").replace("`", "'")
            drift = (
                f"⚠ Status 写 {m.group(1)}/{m.group(2)}，实点 {done}/{total}"
                f"（命中处原文：…{ctx}…）"
            )
        bl = BLOCKED.search(text)
        rows.append(
            {
                "file": path.name,
                "status": status,
                "label": label_of(status),
                "done": done,
                "open": open_,
                "drift": drift,
                # 未收的票才看阻塞关系；已收的写 None 也不会显示
                "blocked": (bl.group(1).strip() if bl else "(票面没写 Blocked by)"),
            }
        )
    return rows


def label_of(status: str) -> str:
    """Status 行里 `·` **之前**的那一段——它才是权威的，后面是散文。

    ⚠⚠ 为什么必须切出标签（2026-10-16 修的真缺陷）：
    旧实现拿整行做子串匹配，于是票 44 的
    「进行中 · …根因**已定位**并修完…两项复核**仍未做**」
    命中了 DONE 里的「已定位」，被判成已收，被归进
    「已收但留着说明项（**不是欠的活**）」——
    **一张自己写着「仍未做」的票，被生成器标成不欠活。**
    这正是本脚本存在的目的（`AGENTS.md`：现状必须是机器生成的）
    被自己破坏掉。

    ⇒ 标签优先于散文。看到「已修」就收工，是拿措辞当状态。
    """
    return status.split("·")[0].strip().lower()


def is_done(row: dict[str, str]) -> bool:
    """已收 = 标签写着已收，**或者**一项未勾都没有。

    ⚠ 两个口径都单用会错：票 25 的 Status 写「✅ 冷启动已验完」但不含
    `done` 字样（按关键字判会漏），反过来有些 Status 写 done 却留着
    条件项未勾（那属于「已收但留说明项」，不是未收）。

    ⚠ 标签优先于关键字：标签若在 `OPEN_LABELS` 里，就**一律未收**，
    哪怕散文里写着「已定位」「已修完」——理由见 `label_of` 的注释。
    反过来标签是 `done` / `部分 done` / `wontfix` 的一律已收，
    不再让散文里的某个词把它推翻。
    """
    if row["label"].startswith(OPEN_LABELS):
        return False
    if row["label"].startswith(CLOSED_LABELS):
        return True
    # 标签不认识（老票写的是散文，例如「冷启动已验完」），才回退到关键字。
    return any(k in row["status"] for k in DONE) or row["open"] == "0"


def render(rows: list[dict[str, str]]) -> str:
    closed = [r for r in rows if is_done(r) and r["open"] == "0"]
    closed_with_notes = [r for r in rows if is_done(r) and r["open"] != "0"]
    open_rows = [r for r in rows if not is_done(r)]
    drifted = [r for r in rows if r["drift"]]

    def cell(r: dict[str, str]) -> str:
        n = int(r["done"]) + int(r["open"])
        return f"| `{r['file']}` | {r['done']}/{n} | {r['status']} |"

    out = [
        "# 进展现状（**机器生成，勿手改**）",
        "",
        "> 由 `scripts/ticketstatus.py` 生成。改完票据就重跑：",
        "> `python scripts/ticketstatus.py`（`--check` 只报漂移不写文件）。",
        "",
        f"票据总数 **{len(rows)}** ｜ 验收项全勾 **{len(closed)}** ｜ "
        f"已收但留有说明项 **{len(closed_with_notes)}** ｜ 未收 **{len(open_rows)}**",
        "",
        "⚠ **「勾数」不等于「进度」**：已收的票里仍可能有未勾项，那些是",
        "**条件项或永久性说明**（例如夹具 gitignored、缺件即跳过）。",
        "反过来，Status 行是**手写**的，会漂——2026-10-15 一晚之间就有三张票的",
        "Status 与自己的验收项矛盾。**冲突时以本表和验收项为准。**",
        "",
        "ℹ **分类看的是 Status 行 `·` 之前那个 triage 标签，不是散文**。",
        "「已定位」「已修完」这类词出现在说明里**不代表这张票已收**——",
        "曾经票 44 写着「进行中 …两项复核仍未做」，却被说明里的「已定位」",
        "判成「不是欠的活」。措辞不是状态，标签才是。",
        "",
        "## 一、未收的票（这里就是要干的活）",
        "",
        "⚠ **这几张剩下的几乎都不是「写代码」**，是人的眼睛与手 —— ",
        "点一次发送、截一张图、看一眼托盘。⚠ 别拿它们当 agent 能接的活。",
        "",
        "| 票 | 勾/总 | Status | Blocked by |",
        "|---|---|---|---|",
    ]
    for r in open_rows:
        n = int(r["done"]) + int(r["open"])
        out.append(f"| `{r['file']}` | {r['done']}/{n} | {r['status']} | {r['blocked']} |")

    out += [
        "",
        "## 二、已收但留着说明项（**不是欠的活**）",
        "",
        "| 票 | 勾/总 | Status |",
        "|---|---|---|",
    ]
    for r in closed_with_notes:
        out.append(cell(r))

    out += ["", "## 三、已收（验收项全勾）", "", "| 票 | 勾/总 |", "|---|---|"]
    for r in closed:
        n = int(r["done"]) + int(r["open"])
        out.append(f"| `{r['file']}` | {r['done']}/{n} |")

    if drifted:
        out += [
            "",
            "## 四、⚠ 脚本当场抓出来的漂移（Status 行自己写的数字与实点不符）",
            "",
            "| 票 | Status 写的 | 机器点的 |",
            "|---|---|---|",
        ]
        for r in drifted:
            n = int(r["done"]) + int(r["open"])
            out.append(f"| `{r['file']}` | {r['drift'].split('，')[0].replace('⚠ Status 写 ', '')} | {r['done']}/{n} |")
        out += [
            "",
            "> 这些不是脚本猜的，是 Status 行里的 `N/M` 与实际勾选数量对不上。",
            "> **按本表的数字读进度**，别按 Status 读。",
        ]

    out.append("")
    return "\n".join(out)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--check", action="store_true", help="report drift, write nothing")
    args = ap.parse_args()

    rows = collect()
    text = render(rows)
    if args.check:
        if not OUT.exists():
            print("DRIFT: STATUS.md does not exist - run: python scripts/ticketstatus.py")
            return 1
        if OUT.read_text(encoding="utf-8") != text:
            print("DRIFT: STATUS.md is stale - run: python scripts/ticketstatus.py")
            return 1
        print("STATUS.md up to date")
        return 0
    OUT.write_text(text, encoding="utf-8", newline="\n")
    open_rows = [r for r in rows if not is_done(r)]
    print(f"wrote {OUT.relative_to(ROOT)}  ({len(rows)} tickets, {len(open_rows)} not closed)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
