---
id: RES-159
title: Match arm guards `case X(n) if n > 0 => ...`
state: OPEN
priority: P3
goalpost: G13
created: 2026-04-17
owner: executor
---

## Summary
Today match arms match on shape only. Adding a guard — a boolean
expr gated by `if` — lets users express "first arm whose pattern
matches AND whose guard is true". The exhaustiveness checker has
to back off politely: guarded arms don't count as covering their
pattern.

## Acceptance criteria
- Parser: `case <pattern> if <expr> => <body>`.
- Semantics: guard evaluated in the pattern's scope (so pattern
  bindings are visible). False guard → fall through to next arm.
- Exhaustiveness (RES-054): a guarded arm is treated as
  non-covering; its pattern is still considered "partially
  covered" (so a following `case _ => ...` is not flagged as
  unreachable).
- Unit tests: guard binding access, false guard falls through,
  exhaustiveness behavior with and without unguarded catch-all.
- Commit message: `RES-159: match arm guards`.

## Notes
- Guard expressions that call impure functions or mutate state:
  allowed, but strongly cautioned against in SYNTAX.md. The
  verifier (G9) will refuse to reason about them.
- No `@` bindings yet — that's RES-161.

## Log
- 2026-04-17 created by manager
