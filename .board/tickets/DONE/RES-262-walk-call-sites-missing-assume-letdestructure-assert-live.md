---
id: RES-262
title: "LSP find-references: walk_call_sites misses Assume, Assert, LetDestructureStruct, LiveBlock, MapLiteral, SetLiteral"
state: DONE
priority: P3
goalpost: G17
created: 2026-04-20
owner: executor
Claimed-by: Claude
---

## Summary

`walk_call_sites` in `resilient/src/lsp_server.rs` is the AST walker that
powers `textDocument/references`. It recursively descends into every node
that can contain a `CallExpression`, but six node variants are silently
dropped in the catch-all `_ => {}` branch:

| Node variant | Sub-expression(s) that can contain calls |
|---|---|
| `Node::Assume { condition, message, .. }` | `condition`, `message` |
| `Node::Assert { condition, message, .. }` | `condition`, `message` |
| `Node::LetDestructureStruct { value, .. }` | `value` |
| `Node::LiveBlock { body, invariants, .. }` | `body`, `invariants` |
| `Node::MapLiteral { entries, .. }` | both key and value expressions in each entry |
| `Node::SetLiteral { items, .. }` | each item expression |

Because these nodes are not descended into, any function call that appears
inside an `assume(...)`, `assert(...)`, struct-destructure RHS, live-block
body or invariant, map literal, or set literal is **invisible to
find-references**. The feature silently returns a false-negative result.

## Affected code

`resilient/src/lsp_server.rs` — `fn walk_call_sites` (line ~467). The
catch-all `_ => {}` arm at line ~623 silently drops these six variants.

## Example that exposes the bug

```resilient
fn helper() -> int { 1 }

fn f(int x) {
    assume(helper() > 0);           // call inside assume — not found today
    assert(helper() == 1);          // call inside assert — not found today
    let Point { x: a, .. } = helper_returns_point(); // RHS — not found today
    let m = { helper(): 1 };        // map key — not found today
}
```

`textDocument/references` on `helper` will return only direct call sites
outside these constructs; calls inside them are missed.

## Acceptance criteria

- `walk_call_sites` gains explicit match arms for each of the six missing
  variants, descending into every sub-expression that can contain a call:
  - `Node::Assume { condition, message, .. }` → walk `condition`; walk
    `message` if `Some`.
  - `Node::Assert { condition, message, .. }` → walk `condition`; walk
    `message` if `Some`.
  - `Node::LetDestructureStruct { value, .. }` → walk `value`.
  - `Node::LiveBlock { body, invariants, backoff, timeout, .. }` → walk
    `body`; walk each invariant.
  - `Node::MapLiteral { entries, .. }` → walk each key and value
    expression.
  - `Node::SetLiteral { items, .. }` → walk each item.
- New unit tests in `lsp_server.rs` (inside `#[cfg(test)]`):
  - `references_finds_call_inside_assume`
  - `references_finds_call_inside_assert`
  - `references_finds_call_inside_let_destructure_rhs`
  - `references_finds_call_inside_map_literal`
  - `references_finds_call_inside_set_literal`
  Each test: write a short source snippet, call `walk_call_sites`, assert
  the expected `Range` is returned.
- Existing find-references tests continue to pass.
- `cargo test` passes with 0 failures.
- `cargo clippy --all-targets -- -D warnings` clean.
- Commit: `RES-262: walk_call_sites — cover Assume, Assert, LetDestructure, LiveBlock, Map, Set`.

## Notes

- `LiveBlock` also has optional `backoff` and `timeout` fields (`BackoffConfig`)
  that do not contain user-written `Node` expressions and do not need to
  be descended into.
- `Node::Assert` definition (line ~1058 of `main.rs`):
  `Assert { condition: Box<Node>, message: Option<Box<Node>>, span }`.
- `Node::Assume` definition (line ~1068):
  `Assume { condition: Box<Node>, message: Option<Box<Node>>, span }`.
- `Node::LetDestructureStruct` definition (line ~1284):
  `LetDestructureStruct { value: Box<Node>, .. }`.
- Do NOT add `Node::StructDecl` or `Node::TypeAlias` — those contain no
  user-written call expressions.
- This is an additive change; no existing test should change its output.

## Log

- 2026-04-20 created by analyzer (`walk_call_sites` catch-all silently
  drops six node variants that can contain nested function calls; find-
  references produces false-negative results for calls in assume/assert/
  let-destructure/live-block/map/set)
