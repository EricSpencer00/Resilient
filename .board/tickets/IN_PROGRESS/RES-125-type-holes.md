---
id: RES-125
title: Type holes `_` in annotations are treated as inference placeholders
state: OPEN
priority: P3
goalpost: G7
created: 2026-04-17
owner: executor
---

## Summary
`let x: _ = 3 + 2` should parse and infer `x: Int`. Useful when the
user wants to assert "there IS a type, I just don't want to write
it" — especially in tutorials. Implementation is trivial once HM is
in: a `_` in a type position becomes a fresh `Type::Var`.

## Acceptance criteria
- Parser accepts `_` as a type: `let x: _`, `fn foo(_ x) -> _`,
  `Array<_>`, etc.
- Each `_` becomes a distinct fresh inference variable; they don't
  unify just because they share a syntactic `_`.
- Display: when the inferer reports an error involving a `_`-origin
  variable, the message reads `type hole at line:col` rather than
  `?t0`.
- Unit tests covering each position (let, param, return, generic
  arg).
- Commit message: `RES-125: `_` type holes as inference placeholders`.

## Notes
- Parameter-position `_` means "infer", not "don't care about the
  value" — don't confuse with pattern holes in `match`.
- If full inference can't pin the hole down at use sites, the
  error message should say `cannot infer type of hole at L:C` with
  the concrete span.

## Log
- 2026-04-17 created by manager
