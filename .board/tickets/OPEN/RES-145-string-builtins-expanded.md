---
id: RES-145
title: String builtins: replace / to_upper / to_lower / format
state: OPEN
priority: P3
goalpost: G11
created: 2026-04-17
owner: executor
---

## Summary
RES-043 added split/trim/contains/substring. The next ergonomic gap
is case conversion, substring replacement, and a minimal format
string function. Lock down a small, predictable surface.

## Acceptance criteria
- `replace(s: String, from: String, to: String) -> String` — all
  occurrences, left-to-right, non-overlapping.
- `to_upper(s: String) -> String` and `to_lower(s: String) ->
  String` — ASCII-only behavior to avoid locale surprises.
  Document the ASCII-only semantics.
- `format(fmt: String, args: Array<?>) -> String` —
  `{}` placeholders consume args in order; `{{` and `}}` escape.
  Unknown / mismatched args → runtime error with span.
- Unit tests: one success + one error path per builtin.
- Typechecker signatures added to the builtin table in
  `typechecker.rs`.
- Commit message: `RES-145: string builtins — replace/upper/lower/format`.

## Notes
- `format` is NOT printf — no width / precision / type specifiers.
  Keep the grammar to `{}` for the MVP; expand in a follow-up if
  needed.
- All functions return new Strings; don't mutate in place — our
  runtime has no mutable-String concept yet.

## Log
- 2026-04-17 created by manager
