# Issue tracker

**Local markdown.** Issues are files under `.scratch/<feature>/`, not a hosted tracker.

## Why

This repo is a solo, greenfield project with no git remote. `gh` is not installed.
Hosting the tracker in-repo keeps issues versionable, reviewable in diffs, and
readable by every skill without extra tooling.

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

## Migrating later

If a GitHub remote is added later, `to-tickets` and `triage` will create real issues
instead. The `.scratch/` files remain as history; move them under
`.scratch/archive/<date>/` once the migration is complete.
