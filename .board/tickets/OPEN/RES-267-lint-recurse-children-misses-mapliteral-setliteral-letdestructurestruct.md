---
id: RES-267
title: "lint.rs: recurse_children misses MapLiteral, SetLiteral, LetDestructureStruct — L0003/L0004/L0006 blind spots"
state: OPEN
priority: P3
goalpost: G11
created: 2026-04-20
owner: executor
---

## Summary

`recurse_children` in `resilient/src/lint.rs` is the generic AST walker
used by lints L0003 (self-comparison), L0004 (mixed-and/or), and L0006
(assume-false). Three node variants are absent from its match, so
sub-expressions inside those constructs are never visited:

| Missing variant | Sub-expressions that should be walked |
|---|---|
| `Node::MapLiteral { entries, .. }` | each key and value expression |
| `Node::SetLiteral { items, .. }` | each item expression |
| `Node::LetDestructureStruct { value, .. }` | the RHS `value` |

## Impact

```resilient
// L0003: self-comparison inside a map literal — not detected today
let m = { x == x: 1 };

// L0004: mixed &&/|| inside a set literal — not detected today
let s = #{ a && b || c };

// L0006: assume(false) inside LetDestructureStruct RHS — not detected today
let Point { x } = f(assume(false));
```

## Affected code

`resilient/src/lint.rs` — `fn recurse_children` (line ~742).
The catch-all `_ => {}` arm at line ~873 silently drops these three variants.

## Acceptance criteria

- `recurse_children` gains explicit arms for each of the three missing
  variants, descending into every sub-expression:
  - `Node::MapLiteral { entries, .. }` → `f(k); f(v)` for each `(k, v)`.
  - `Node::SetLiteral { items, .. }` → `f(item)` for each item.
  - `Node::LetDestructureStruct { value, .. }` → `f(value)`.
- New unit tests in `lint.rs` `#[cfg(test)]` (inside the `tests` module):
  - `l0003_fires_inside_map_literal_key`
  - `l0004_fires_inside_set_literal`
  - `l0006_fires_inside_let_destructure_rhs`
- Existing lint tests continue to pass.
- `cargo test` passes with 0 failures.
- `cargo clippy --all-targets -- -D warnings` clean.
- Commit: `RES-267: recurse_children — add MapLiteral, SetLiteral, LetDestructureStruct arms`.

## Notes

`collect_identifier_reads_in` (used by L0001) already handles all three
variants correctly (see lint.rs lines 362–377). This ticket is specifically
about `recurse_children` which powers L0003, L0004, and L0006.

This is an additive change — no existing test should change output.

## Log

- 2026-04-20 created by analyzer
