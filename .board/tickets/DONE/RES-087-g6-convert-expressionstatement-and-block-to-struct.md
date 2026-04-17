---
id: RES-087
title: G6 convert ExpressionStatement + Block to struct variants
state: DONE
priority: P1
goalpost: G6
created: 2026-04-17
owner: executor
---

## Summary
Two final tuple-variant conversions to finish the tuple-to-struct
migration for G6 leaves: `ExpressionStatement(Box<Node>)` (13
sites) and `Block(Vec<Node>)` (20 sites). After this, only the
large structural variants (`Match`, `StructLiteral`,
`FunctionLiteral`, `Function`, `LiveBlock`, `Assert`, `StructDecl`,
`Use`) remain for full G6 closure — those are tracked as RES-089
when the time comes.

## Acceptance criteria
- `Node::ExpressionStatement(Box<Node>)` →
  `Node::ExpressionStatement { expr: Box<Node>, span: Span }`.
- `Node::Block(Vec<Node>)` →
  `Node::Block { stmts: Vec<Node>, span: Span }`.
- Both `span` fields `#[allow(dead_code)]` matching the
  RES-078/079/084/085/086 convention.
- Parser `parse_block_statement` captures the `{` token's span
  before advancing.
- Parser `parse_expression_statement` captures the current-token
  span before parsing the expression.
- Every destructure / construction site updated across `main.rs`,
  `typechecker.rs`, `verifier_z3.rs`, `compiler.rs`. Expect ~33
  sites total.
- Fallback constructions like `Box::new(Node::Block(Vec::new()))`
  become `Box::new(Node::Block { stmts: Vec::new(), span: Span::default() })`.
- New unit test: parse `{ let x = 1; let y = 2; }` inside a fn
  body and assert the resulting `Block` has two stmts and a
  non-default span.
- `cargo build`, `cargo test`, `cargo clippy -- -D warnings` pass
  on all three feature configs (default, `--features z3`,
  `--features lsp`).
- Commit message: `RES-087: tuple→struct for ExpressionStatement + Block (G6 partial)`.

## Notes
- `Block` has 20 sites — the largest tuple variant migration. Most
  are in interpreter / compiler dispatch. The typical pattern is
  `Node::Block(stmts) => ...` which becomes
  `Node::Block { stmts, .. } => ...` — mechanical.
- `ExpressionStatement(expr)` lives in many nested constructors,
  e.g. `Some(Node::ExpressionStatement(Box::new(expr)))` — each
  needs a span threaded in. Capture at `parse_expression_statement`
  entry.
- After this ticket, ROADMAP G6 cell should note only the
  **structural** variants remain (Match/StructLiteral/etc).

## Log
- 2026-04-17 created by manager
- 2026-04-17 acceptance criteria filled in by manager (orchestrator pass)
- 2026-04-17 executor landed:
  - `Node::Block(Vec<Node>)` →
    `Node::Block { stmts: Vec<Node>, span: Span }`.
  - `Node::ExpressionStatement(Box<Node>)` →
    `Node::ExpressionStatement { expr: Box<Node>, span: Span }`.
  - Both `span` fields `#[allow(dead_code)]` matching the
    convention.
  - `parse_block_statement` captures the `{` token's span before
    advancing.
  - `parse_expression_statement` captures the current-token span
    before parsing.
  - `parse_maybe_index_assignment` and other ExpressionStatement
    construction sites use `Span::default()` for synthetic nodes.
  - All ~33 destructure / construction sites updated across
    `main.rs`, `typechecker.rs`, `compiler.rs` via targeted sed
    + hand-patches for the construction sites.
- 2026-04-17 tests:
  - `block_and_expression_statement_carry_spans` parses
    `fn f() { let x = 1; let y = 2; }`, asserts the inner Block
    has 2 stmts and a non-default span; then parses `1 + 2;`,
    asserts the wrapping ExpressionStatement has a non-default
    span.
- 2026-04-17 verification across three feature configs:
  - default: 214 unit + 1 golden + 11 smoke = 226 tests
  - `--features z3`: 222 + 1 + 12 = 235
  - `--features lsp`: 217 + 1 + 11 = 229
  All three `cargo clippy -- -D warnings` clean.
- ROADMAP G6 cell updated. Tuple-variant work is now COMPLETE.
  Remaining for full G6 closure: structural variants
  (`Match`, `StructLiteral`, `FunctionLiteral`, `Function`,
  `LiveBlock`, `Assert`, `StructDecl`, `Use`) — file as RES-088
  when ready.
