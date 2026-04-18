---
id: RES-149
title: `Set<T>` native value type, mirror of Map
state: IN_PROGRESS
priority: P3
goalpost: G11
created: 2026-04-17
owner: executor
---

## Summary
With Map landing in RES-148, Set is cheap to add and covers the
"deduplicate this" pattern directly.

## Acceptance criteria
- `Value::Set(HashSet<Value>)` (std) / `BTreeSet<Value>` (no_std
  alloc).
- Literal syntax: `#{1, 2, 3}`. Empty set: `#{}`.
- Builtins: `set_new()`, `set_insert(s, x)`, `set_remove(s, x)`,
  `set_has(s, x) -> Bool`, `set_len(s) -> Int`, `set_items(s) ->
  Array<T>`.
- Same key-type restriction as RES-148 (Int / String / Bool).
- Unit tests covering each builtin.
- Commit message: `RES-149: Set<T> native value type`.

## Notes
- Iteration order: document as "unspecified on std; sorted on
  no_std". Callers shouldn't depend on it; the split is a
  consequence of stdlib choices.
- `set_items` is lifted out into an array so the language's
  array-consuming primitives (`for ... in`, array comprehensions
  via RES-156) work on sets without extra syntax.

## Log
- 2026-04-17 created by manager
- 2026-04-17 claimed by executor
