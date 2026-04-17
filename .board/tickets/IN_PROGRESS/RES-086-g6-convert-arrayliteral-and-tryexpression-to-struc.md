---
id: RES-086
title: G6 convert ArrayLiteral + TryExpression to struct variants
state: OPEN
priority: P1
goalpost: G6
created: 2026-04-17
owner: executor
---

## Summary
G6 is mostly green — leaves, statements, core expressions, and
index/field ops all carry spans. Four remaining variants are still
in **tuple form** (`ArrayLiteral(Vec<Node>)`,
`ExpressionStatement(Box<Node>)`, `Block(Vec<Node>)`,
`TryExpression(Box<Node>)`) and can't add a span without being
converted to struct variants first.

This ticket does the two smallest of those four — `ArrayLiteral`
(7 references) and `TryExpression` (3 references). The larger two
(`ExpressionStatement` with 13 references, `Block` with 20) are
separate tickets (RES-087, RES-088) so each piece stays reviewable.

## Acceptance criteria
- `Node::ArrayLiteral(Vec<Node>)` →
  `Node::ArrayLiteral { items: Vec<Node>, span: Span }`.
- `Node::TryExpression(Box<Node>)` →
  `Node::TryExpression { expr: Box<Node>, span: Span }`.
- `span` field in each marked `#[allow(dead_code)]` with a comment
  noting "consumed in follow-ups" — matches the RES-078/079/084/085
  convention.
- Parser populates the span:
  - `parse_array_literal` captures the `[` token's span before
    advancing.
  - The postfix `?` expansion in `parse_expression` captures the
    `?` token's span.
- Every destructure / construction site across `main.rs`,
  `typechecker.rs`, `verifier_z3.rs`, `compiler.rs` updated. ~10
  call sites total.
- New unit test: parse `[1, 2, 3]`, assert the `ArrayLiteral`'s
  span has `start.line >= 1`.
- `cargo build`, `cargo test`, `cargo clippy -- -D warnings` pass
  on all three feature configs (default, `--features z3`,
  `--features lsp`).
- Commit message: `RES-086: tuple→struct for ArrayLiteral + TryExpression (G6 partial)`.

## Notes
- Parser sites:
  - `parse_array_literal` around `main.rs:1956`
  - Postfix `?` handling in `parse_expression`
- Fallback-on-error constructions like
  `unwrap_or(Node::ArrayLiteral(Vec::new()))` become
  `unwrap_or(Node::ArrayLiteral { items: Vec::new(), span: Span::default() })`.
- Match sites typically look like `Node::ArrayLiteral(items) => ...`
  — convert to `Node::ArrayLiteral { items, .. } => ...`.
- After this ticket ships, file RES-087 (ExpressionStatement) and
  RES-088 (Block) to complete the tuple-variant sweep.

## Log
- 2026-04-17 created by manager
- 2026-04-17 acceptance criteria filled in by manager (orchestrator pass)
