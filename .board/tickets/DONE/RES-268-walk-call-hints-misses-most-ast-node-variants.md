---
id: RES-268
title: "LSP inlay hints: walk_call_hints misses most AST node variants — parameter hints absent in loops, match, assignment, etc."
state: OPEN
priority: P3
goalpost: G17
created: 2026-04-20
owner: executor
Claimed-by: Claude
closed-commit: 04d966897792ef95f6a954f29213705669459b89
---

## Summary

`walk_call_hints` in `resilient/src/lsp_server.rs` (the walker behind
`textDocument/inlayHints` parameter hints) covers only a small subset of
AST nodes:

**Currently handled**: `Program`, `Function`, `Block`, `CallExpression`,
`LetStatement`, `StaticLet`, `ReturnStatement`, `IfStatement`,
`InfixExpression`, `PrefixExpression`

**Silently dropped** (catch-all `_ => {}`):

| Missing variant | Sub-expressions that contain calls |
|---|---|
| `Node::WhileStatement { condition, body }` | both |
| `Node::ForInStatement { iterable, body }` | both |
| `Node::Assignment { value }` | value |
| `Node::ExpressionStatement { expr }` | expr |
| `Node::IndexExpression { target, index }` | both |
| `Node::IndexAssignment { target, index, value }` | all three |
| `Node::FieldAccess { target }` | target |
| `Node::FieldAssignment { target, value }` | both |
| `Node::TryExpression { expr }` | expr |
| `Node::ArrayLiteral { items }` | each item |
| `Node::StructLiteral { fields }` | each field value |
| `Node::Match { scrutinee, arms }` | scrutinee + each guard + each body |
| `Node::FunctionLiteral { body, .. }` | body, requires, ensures |
| `Node::ImplBlock { methods }` | each method |
| `Node::Assume { condition, message }` | both |
| `Node::Assert { condition, message }` | both |
| `Node::LetDestructureStruct { value }` | value |
| `Node::LiveBlock { body, invariants }` | body + each invariant |
| `Node::MapLiteral { entries }` | each key + each value |
| `Node::SetLiteral { items }` | each item |

A function call anywhere inside a `while` loop body, `for` loop, `match`
arm, or any of the other missed constructs gets no parameter inlay hint.

## Affected code

`resilient/src/lsp_server.rs` — `fn walk_call_hints` (line ~1058).
The comment "Expand cases as new AST shapes need hint coverage" at line
~1150 explicitly acknowledges this is incomplete.

## Acceptance criteria

- `walk_call_hints` gains explicit match arms for every variant listed in
  the "Silently dropped" table above, recursing into every sub-expression
  that can contain a `CallExpression`.
- New unit tests in `lsp_server.rs` `#[cfg(test)]`:
  - `inlay_hints_fire_inside_while_body`
  - `inlay_hints_fire_inside_for_body`
  - `inlay_hints_fire_inside_match_arm`
  - At least one test for a missing variant from each category
    (statement, expression, literal).
- Existing inlay-hint tests continue to pass.
- `cargo test` passes with 0 failures.
- `cargo clippy --all-targets -- -D warnings` clean.
- Commit: `RES-268: walk_call_hints — cover remaining AST node variants`.

## Notes

`collect_top_level_fns` (used to build the `fns` map) only collects
top-level `Function` nodes, not impl methods. That is a separate gap
tracked by RES-266; this ticket focuses only on the walker coverage.

## Log

- 2026-04-20 created by analyzer
