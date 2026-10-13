# Issue tracker

**Local markdown.** Issues are files under `.scratch/<feature>/`, not a hosted tracker.

## Why

Hosting the tracker in-repo keeps issues versionable, reviewable in diffs, and
readable by every skill without extra tooling.

⚠ **2026-10-13 更正。** 本节原文写的是「no git remote, `gh` is not installed」，
写于项目还没上 GitHub 的时候。现在仓库**在 GitHub 上**
（`SeasonTemple/chat-servant`，有 `.github/workflows/` 与 PR 模板），
`gh` 也可用了。

但票据**仍然留在仓库里**，理由变了——不是「没有托管方案」，
而是这三个：

1. 票据与代码在同一个 diff 里，review 时看得见「这张票为什么改这些行」；
2. `.scratch/` 里有本仓库专用的字段（`Blocked by` 的阻塞边、`Status` 的分诊标签），
   比 GitHub Issue 表单能表达的更结构化；
3. agent 与人读的是同一份文件，不需要在两套工具之间同步。

## How issues are stored

```
.scratch/
  <feature-slug>/
    FEATURE.md            # the spec / problem statement the tickets slice up
    issues/
      01-<slug>.md        # one file per ticket
      02-<slug>.md
```

Tickets are numbered from `01` in dependency order, so the blocking edges read
top to bottom. Each ticket file uses this shape:

```markdown
# 01: <short title>

**What to build:** the end-to-end behaviour this makes work, from the user's
perspective — not a layer-by-layer implementation list.

**Blocked by:** the numbers/titles that gate this one, or "None (can start
immediately)".

**Status:** ready-for-agent

- [ ] <observable, testable acceptance criterion>
- [ ] <observable, testable acceptance criterion>
```

## Rules

- **Numbering is sequential and permanent.** Never renumber, never reuse a number,
  never reorder. A cancelled ticket keeps its number and moves to
  `Status: cancelled`.
- **Every ticket states its blocking edges explicitly.** A ticket with no real
  blocker says so — otherwise the frontier is unknowable.
- **One feature per directory.** If a ticket does not belong to an existing feature
  directory, create a new one rather than dropping it into an unrelated folder.
- **Acceptance criteria are checkable.** "Works well" is not acceptance. "WGC 帧在
  微信窗口被其他窗口遮挡时仍能取到，且不出现黑帧" is.
- **Never edit another agent's in-flight issue file.** Append a `## Update <date>`
  section instead, so attribution stays visible.

## If GitHub Issues is ever used

`.scratch/` is the tracker of record. If GitHub Issues is ever adopted for
public-facing bugs, keep `.scratch/` for design tickets: it carries
`Blocked by` edges and triage labels that the Issue form does not model.
The `.scratch/` files stay as history either way; move them under
`.scratch/archive/<date>/` only at a real migration.
