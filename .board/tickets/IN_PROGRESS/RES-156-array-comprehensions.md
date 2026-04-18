---
id: RES-156
title: Array comprehensions `[f(x) for x in xs if p(x)]`
state: IN_PROGRESS
priority: P3
goalpost: G12
created: 2026-04-17
owner: executor
---

## Summary
With map + set + array filter/transform being common, give users
a single sugar that handles the common case: one-dim
comprehensions with optional filter. Desugars to a simple for-loop
+ push at parse time.

## Acceptance criteria
- Syntax: `[<expr> for <binding> in <iterable> (if <guard>)?]`.
- Desugars to:
  ```
  { let _r = []; for <binding> in <iterable> { if (<guard>) { push(_r, <expr>); } } _r }
  ```
- Works on Arrays and Sets (`set_items` result, RES-149).
- Unit tests: simple map, map+filter, nested-scoped binding doesn't
  leak.
- Golden example `examples/comprehension_demo.rs`.
- Commit message: `RES-156: array comprehensions`.

## Notes
- Don't support multi-clause `for` in the comprehension
  (`[x for x in xs for y in ys]`) — that's a rabbit hole of
  performance surprises. One `for`, one optional `if`.
- The desugared form MUST use a fresh name (`_r$0`, `_r$1`, ...) to
  avoid shadowing user bindings if the expr references an outer
  `_r`.

## Log
- 2026-04-17 created by manager
- 2026-04-17 claimed by executor
