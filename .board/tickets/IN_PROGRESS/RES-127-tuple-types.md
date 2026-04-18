---
id: RES-127
title: Tuple types `(Int, String)` with inference and destructuring let
state: OPEN
priority: P3
goalpost: G7
created: 2026-04-17
owner: executor
---

## Summary
Today "return two things" means packing a struct or an array. Tuples
are a lighter-weight alternative — especially for multi-value
returns — and HM inference handles them naturally as type
constructors of fixed arity.

## Acceptance criteria
- Parser: `(a, b)` in expression position is a tuple literal.
  `(Int, String)` in type position is a tuple type. Unit tuple `()`
  = `Type::Void` alias (same thing).
- Indexing: `t.0`, `t.1` for positional access (parser extension to
  dotted access so field-style works on tuples too).
- Destructuring let: `let (x, y) = foo();`.
- Interpreter + VM + JIT: tuples represented as a thin `Vec<Value>`
  for now; optimize layout in a follow-up.
- Unit tests covering literal construction, indexing, destructuring,
  and a type error on arity mismatch.
- Commit message: `RES-127: tuple types + destructuring let`.

## Notes
- One-element tuple syntax is `(x,)` with a trailing comma, per the
  Rust convention; `(x)` is just a parenthesized expression. Make
  sure the parser distinguishes.
- Don't introduce a `first`/`second` stdlib yet — `.0` / `.1`
  covers it.

## Log
- 2026-04-17 created by manager
