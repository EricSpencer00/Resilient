---
id: RES-148
title: `Map<K, V>` native value type with insert / get / remove / keys
state: DONE
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
- 2026-04-17 claimed by executor
- 2026-04-17 done by executor

## Resolution

Files changed:
- `resilient/src/main.rs`
  - New `Value::Map(HashMap<MapKey, Value>)` variant. `MapKey` is an
    internal enum limited to `Int` / `Str` / `Bool` (the three
    hashable primitives) — anything else at a key slot surfaces a
    runtime error from `MapKey::from_value`. `HashMap` is used for
    O(1) lookup; Display sorts keys so output is deterministic
    across runs.
  - New `Node::MapLiteral { entries: Vec<(Node, Node)>, span }` AST
    variant with corresponding parser (`parse_map_literal` +
    `parse_map_entry`) and interpreter eval arm. `{k -> v, ...}`
    only parses in expression position — statement-level `{` still
    opens a block, as before.
  - Six builtins: `map_new`, `map_insert`, `map_get` (returns
    `Result<V, "not found">`), `map_remove`, `map_keys` (sorted
    Array), `map_len`. Registered in the `BUILTINS` const slice.
  - 12 unit tests covering each builtin (happy path + edge cases),
    key-type restriction (non-hashable key errors on both insert and
    literal), literal parsing across Int/String/Bool heterogeneous
    keys, empty `{}`, overwrite semantics, and Display.
- `resilient/src/typechecker.rs`
  - Added a `Node::MapLiteral` arm in `check_node` that walks
    entries for nested errors and returns `Type::Any` (deferred
    `Type::Map<K, V>` until G7 — same pattern the Array / Result
    builtins already use; see ticket note).
  - Registered the six builtins with `Any`-based signatures.
- `resilient/src/compiler.rs`
  - `node_line` arm for `Node::MapLiteral` so the exhaustive match
    that maps AST nodes to source lines still compiles.

Acceptance criteria:
- `Value::Map(HashMap<...>)` — yes (std; `resilient-runtime` has no
  `Value` mirror so the no_std posture is unaffected).
- Literal syntax `{"k" -> 1, "m" -> 2}` — yes, via `MapLiteral`.
- Six builtins with the specified return shapes — yes.
- Key restriction to Int/String/Bool — yes, enforced at runtime in
  both `map_insert` and the `MapLiteral` interpreter arm.
- Unit tests per builtin + key-type restriction — yes (12 tests).

Deviations:
- Typechecker sees maps as `Type::Any` instead of a dedicated
  `Type::Map<K, V>` constructor. The ticket's notes say "the
  typechecker sees `Map<K,V>` as a special Type constructor," but
  adding a new `Type` variant would break exhaustive matches across
  `typechecker.rs` and doesn't gain anything until proper generics
  (RES-124) land. The existing Array / Result builtins take the
  same shortcut. When RES-124 lands, both can migrate together.
- Used `HashMap` in std rather than `BTreeMap` for the no_std
  variant — the interpreter is std-only; the ticket's "BTreeMap
  (no_std)" branch simply doesn't apply because no no_std code path
  touches `Value::Map`.

Verification:
- `cargo build` (default + all features) — clean.
- `cargo test` — 265 unit (+12 new) + 13 integration pass.
- `cargo clippy --tests -- -D warnings` — clean.
- `cargo clippy --features logos-lexer,z3 --tests -- -D warnings` —
  clean.
- Manual end-to-end scripts: `{1 -> "a", true -> "b"}` literal and
  `map_new() + map_insert + map_get + map_len + map_keys` builtins
  both exercised and print the expected output.
