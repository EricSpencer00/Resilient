---
id: RES-239
title: "All lint passes (L0001–L0005) silently skip methods inside `impl` blocks"
state: DONE
priority: P2
goalpost: G10
created: 2026-04-20
owner: executor
claimed-by: Claude
---

## Summary

The five lint passes in `resilient/src/lint.rs` only descend into
top-level `Node::Function` nodes. `Node::ImplBlock { methods, .. }`
is never walked by any of the `run_l000N_*` functions or their
helpers (`walk_matches`, `walk_self_comparisons`, `walk_and_or`,
`walk_unused_return`). As a result:

- Unused local bindings inside a method → no L0001 warning.
- Unreachable match arms inside a method → no L0002 warning.
- Self-comparisons (`x == x`) inside a method → no L0003 warning.
- Mixed `&&`/`||` without parens inside a method → no L0004 warning.
- Redundant trailing `return;` inside a method → no L0005 warning.

A user who writes all their code inside `impl` blocks gets zero lint
coverage.

## Reproduction

```resilient
struct Point { x: Int, y: Int }

impl Point {
    fn debug(self) {
        let unused = 99;    // should fire L0001 — does not
        return;             // should fire L0005 — does not
    }
}
```

## Acceptance criteria

- Each `run_l000N_*` helper (or its inner walker) gains a branch for
  `Node::ImplBlock { methods, .. }` that iterates over `methods` and
  recurses into each method body as if it were a top-level function.
- The top-level `run_l0001_unused_local` helper must also collect
  method bodies when iterating over top-level statements.
- Unit tests (new, not modifying existing tests):
  - L0001 fires for an unused binding inside a method body.
  - L0002 fires for an unreachable match arm inside a method body.
  - L0005 fires for a trailing `return;` inside a method body.
  - Existing tests continue to pass.
- `cargo test` remains fully green.
- `cargo clippy --all-targets -- -D warnings` remains clean.
- Commit message: `RES-239: extend lint passes (L0001–L0005) to walk impl block methods`.

## Affected code

- `resilient/src/lint.rs`:
  - `run_l0001_unused_local` / `collect_lets_in` /
    `collect_identifier_reads_in`
  - `walk_matches` (L0002)
  - `walk_self_comparisons` (L0003)
  - `walk_and_or` (L0004)
  - `walk_unused_return` (L0005)

## Notes

- Do **not** modify existing tests — add only new ones.
- The fix is purely additive (new `match` arms for `Node::ImplBlock`).
- `ImplBlock` methods are already name-mangled by the parser (e.g.
  `Point::debug` becomes `Point__debug`); no special name handling
  is required beyond recursing into the method body.
- This is a good companion to RES-237 which fixes other missing
  node coverage in the same file.

## Log
- 2026-04-20 created by analyzer (found during review of lint.rs walker coverage)
