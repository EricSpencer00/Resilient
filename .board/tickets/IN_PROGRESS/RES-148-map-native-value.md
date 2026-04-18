---
id: RES-148
title: `Map<K, V>` native value type with insert / get / remove / keys
state: OPEN
priority: P2
goalpost: G11
created: 2026-04-17
owner: executor
---

## Summary
Arrays (RES-032) cover sequential data. Associative data is the
obvious next addition — lookup tables, config dicts, rate counters
keyed by something. Ship as a native value with a small surface of
builtins; worry about user-defined generic containers later.

## Acceptance criteria
- `Value::Map(HashMap<Value, Value>)` (std) /
  `BTreeMap<Value, Value>` (no_std, alloc feature).
- Literal syntax: `{"k" -> 1, "m" -> 2}` (fat arrow avoids conflict
  with struct literals).
- Builtins: `map_new()`, `map_insert(m, k, v)`, `map_get(m, k) ->
  Result<V, Err>` (absent key is `Err("not found")`), `map_remove(m,
  k)`, `map_keys(m) -> Array<K>`, `map_len(m) -> Int`.
- Key type restriction: `Int`, `String`, `Bool` only (nothing that
  breaks `Eq` + `Hash`). Anything else at a key slot is a type
  error.
- Unit tests covering each builtin + the key-type restriction.
- Commit message: `RES-148: Map<K,V> native value type`.

## Notes
- Value identity for maps: use structural equality, not
  reference. Two maps with the same (K,V) pairs compare equal.
- Not gated on full generics (RES-124) — the typechecker sees
  `Map<K,V>` as a special Type constructor.

## Log
- 2026-04-17 created by manager
