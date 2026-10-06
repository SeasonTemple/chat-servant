# Domain docs

**Layout: single-context.** One `CONTEXT.md` at the repo root, ADRs in `docs/adr/`.

There is no monorepo signal (no workspace file, no populated multi-package tree), so
the multi-context `CONTEXT-MAP.md` layout would only add indirection.

## Consumer rules

When a task touches terminology, architecture boundaries, or a decision that was
hard to reach:

1. **Read `CONTEXT.md` before writing code.** It defines the ubiquitous language
   (`Frame`, `ChatPane`, `Message`, `Candidate`, `Intent`, `Session`) and the
   hard constraints. Using a term differently than `CONTEXT.md` defines it is a
   defect, even if the code compiles.
2. **Check `docs/adr/` for an existing decision** on the thing you are about to
   change. ADRs are immutable except by superseding; never edit one in place.
3. **Write an ADR** when a decision meets any of:
   - it constrains what the code may do later (a rule, not a preference),
   - it has a real cost that a future reader would otherwise question,
   - it was hard to decide (there was a genuine trade-off).

   An ADR records the rejected alternatives, not just the choice.
4. **Update `CONTEXT.md`** when a new domain term appears or an existing term's
   meaning changes. Not for implementation notes.
5. **Versioned concepts belong in ADRs**, not `CONTEXT.md`. `CONTEXT.md` is the
   present tense of the domain; ADRs are the past tense of the decisions.

## When to skip

Single-line notes, bug fixes, and anything that introduces no new domain language
and no new constraint do not need a doc change. Writing an ADR for every choice
makes the directory unreadable.
