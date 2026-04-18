---
id: RES-149
title: `Set<T>` native value type, mirror of Map
state: DONE
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
- 2026-04-17 done by executor

## Resolution
- `resilient/src/main.rs`:
  - New `Token::HashLeftBrace` variant, emitted by the hand-rolled
    lexer when seeing `#` followed immediately by `{`. A lone `#`
    still falls through to `Token::Unknown('#')`. Display string
    is `` `#{` ``.
  - New `Node::SetLiteral { items: Vec<Node>, span }` AST variant.
  - New `Value::Set(HashSet<MapKey>)` — reuses the RES-148
    `MapKey` wrapper so the hashable-primitive restriction
    (Int / String / Bool) is enforced by the same code path that
    guards Map keys. Error messages say "Set element" via a
    targeted `.replace("Map key", "Set element")` so diagnostics
    stay on-topic.
  - Parser: `parse_set_literal()` — mirrors `parse_map_literal`
    (comma-separated, trailing comma allowed, empty literal
    `#{}`). Dispatched from `parse_expression` on
    `Token::HashLeftBrace`.
  - Eval: new `Node::SetLiteral` arm evaluates each item and
    inserts via `MapKey::from_value`; duplicates are collapsed.
  - Display: `Value::Set` prints as `#{1, 2, 3}` with items
    sorted for determinism (same rationale as Map's sorted
    Display — otherwise HashSet's random iteration would flake
    golden tests).
  - Six new builtins, all registered in the `BUILTINS` table:
    - `set_new() -> Set`
    - `set_insert(s, x) -> Set` — immutable (clones)
    - `set_remove(s, x) -> Set` — silent no-op on absent
    - `set_has(s, x) -> Bool`
    - `set_len(s) -> Int`
    - `set_items(s) -> Array` — deterministic sort order
- `resilient/src/lexer_logos.rs`:
  - New `#[token("#{")] HashLBrace` tok + `convert` arm mapping
    to `Token::HashLeftBrace`. Logos prefers the longer match
    automatically; a lone `#` stays unmatched.
- `resilient/src/typechecker.rs`:
  - `Node::SetLiteral` arm added to the exhaustive `check_node`
    match; walks each item and returns `Type::Any` (same
    permissive posture as MapLiteral).
  - Prelude env entries for the six `set_*` builtins following
    the Any-typed Map precedent.
- `resilient/src/compiler.rs`:
  - `Node::SetLiteral` arm added to `node_line`'s exhaustive
    span accessor.
- Deviations from ticket: none on the std path. The ticket's
  "no_std alloc → BTreeSet<Value>" is noted in the `Value::Set`
  doc-comment as a follow-up when the `resilient-runtime`
  sibling crate grows a Set value type — that crate has no
  value plane for collections today, so the bifurcation is
  wording-only at this point.
- Unit tests (14 new, covering every acceptance-criteria
  builtin at least once plus literal syntax):
  - `set_new_is_empty` / `set_new_rejects_arguments`
  - `set_insert_adds_and_dedups`
  - `set_insert_rejects_non_hashable_element` (Float rejected)
  - `set_insert_rejects_non_set_first_arg`
  - `set_has_reports_membership` / `set_has_rejects_non_set_first_arg`
  - `set_remove_drops_element_and_ignores_missing`
  - `set_remove_rejects_wrong_arity`
  - `set_len_counts_entries`
  - `set_items_returns_sorted_array`
  - `set_literal_parses_and_evaluates` — end-to-end parser →
    interpreter with duplicate collapse
  - `empty_set_literal_parses` — `#{}` path
  - `set_literal_rejects_float_element_at_runtime` — MapKey
    restriction surfaces through literal eval
- Smoke (manual):
  - `let s = #{1, 2, 3}; println(set_has(s, 2)); …` prints
    `true / false / 3 / #{1, 2, 3, 4} / [1, 2, 3, 4]` — end-to-
    end across parser, evaluator, builtins, and Display.
- Verification:
  - `cargo test --locked` — 377 passed (was 363 before RES-149)
  - `cargo test --locked --features logos-lexer` — 378 passed
  - `cargo clippy --locked --features logos-lexer,z3 --tests
    -- -D warnings` — clean
