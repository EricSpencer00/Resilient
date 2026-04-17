---
id: RES-160
title: Or-patterns in match arms `case 0 | 1 | 2 => ...`
state: OPEN
priority: P3
goalpost: G13
created: 2026-04-17
owner: executor
---

## Summary
Or-patterns collapse clusters of equivalent arms into one line.
Exhaustiveness treats them as the union of the covered spaces,
which is natural in the coalescing algorithm from RES-129.

## Acceptance criteria
- Parser: `<pattern> | <pattern> | ...` at the top of a match arm.
  Lower precedence than struct/tuple destructuring — parens required
  to combine.
- Bindings: if any branch binds a name, ALL branches must bind the
  same set of names to the same types. Otherwise a typecheck
  error: `or-pattern branches bind different names`.
- Exhaustiveness: union the covered space of each branch.
- Unit tests: numeric or-pattern, string or-pattern, mismatched
  bindings error.
- Commit message: `RES-160: or-patterns in match arms`.

## Notes
- Same-binding constraint matches Rust's semantics and avoids
  user confusion about "which branch was taken".
- Don't support or-patterns in `let` bindings yet — match-only
  surface for this ticket.

## Log
- 2026-04-17 created by manager
