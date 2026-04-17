---
id: RES-069
title: G6 add Span to every AST node
state: DONE
priority: P0
goalpost: G6
created: 2026-04-17
owner: executor
---

## Summary
G6 demands "AST hardening": one canonical AST in which every node carries
source-position information. Today the `Node` enum in
`resilient/src/main.rs:456` has no spans — diagnostics that originate after
parsing (typechecker errors, runtime traps, verifier failures) cannot point
back at file:line:col. This ticket introduces a `Span { start: Pos, end: Pos }`
type and threads it through the AST so future error messages can quote the
exact source range.

This is the minimum hardening required before we delete `parser.rs` (RES-070)
and before LSP work (RES-074) can surface diagnostics in an editor.

## Acceptance criteria
- A new `Span` struct (with `start: Pos`, `end: Pos`, where `Pos` is `{ line: usize, column: usize, offset: usize }`) is defined in `resilient/src/main.rs` (or a new `span.rs` module).
- The lexer emits a `(Token, Span)` pair for every token; the existing `Token` enum is unchanged.
- Every variant of `Node` either carries a `span: Span` field directly OR is wrapped in a `Spanned<Node>` newtype. Pick one strategy and apply it uniformly — no half-spans.
- The parser populates the span for every node it constructs.
- At least one diagnostic path (suggest: typechecker mismatch in `typechecker.rs`) is updated to use the span and prints `file:line:col` in its error message.
- A new unit test asserts that a parsed `let x = 1 + 2;` yields a `LetStatement` whose span covers the entire statement and whose value's span covers `1 + 2`.
- `cargo build` succeeds with no new warnings.
- `cargo test` passes — all existing tests still green.
- `cargo clippy -- -D warnings` exits 0.

## Notes
- AST is in `resilient/src/main.rs:456` — `enum Node`.
- Lexer is in `resilient/src/main.rs:98` — `struct Lexer`.
- Existing parser is in `resilient/src/main.rs:608` — `struct Parser` (NOT the standalone `parser.rs`, which is dead code targeted by RES-070).
- `Node::Program(Vec<Node>)` and `Node::Block(Vec<Node>)` are tuple variants. To add a span, convert them to struct variants like `Program { stmts: Vec<Node>, span: Span }`.
- Pattern matches across the codebase will need updating. Run `cargo build` after each variant migration to catch fallout.
- Keep the diff under ~400 lines if possible. If it balloons, split — open a follow-up for the diagnostic-rewriting half.

## Log
- 2026-04-17 created by manager
- 2026-04-17 acceptance criteria filled in by manager (orchestrator pass)
- 2026-04-17 executor partial (foundation): `span.rs` module added with
  `Pos`, `Span`, and `Spanned<T>` types + 7 unit tests (all green).
  Lexer gains `next_token_with_span()` helper that pairs each token
  with its source span. AST migration (touching every `Node` variant +
  every match expression in `main.rs`) and the typechecker diagnostic
  rewrite still TODO — next executor iteration. Default `cargo clippy`
  remains clean; `cargo test` reports 143 unit + 1 golden + 4 smoke
  passing (up from 136 thanks to new span tests).
- 2026-04-17 umbrella CLOSED. Substance delivered through the
  RES-077..088 series across iterations 8-21:
  - RES-077: `Node::Program` statements wrapped in `Spanned<Node>`
  - RES-078: leaf nodes (Int/Float/String/Bool/Identifier)
  - RES-079: core statements (Let/Static/Assignment/Return/If/While/ForIn)
  - RES-080: typechecker errors prefix `<file>:<line>:<col>:`
  - RES-084: core expressions (Prefix/Infix/Call)
  - RES-085: index + field (IndexExpression/IndexAssignment/FieldAccess/FieldAssignment)
  - RES-086: tuple→struct ArrayLiteral + TryExpression
  - RES-087: tuple→struct ExpressionStatement + Block
  - RES-088: structural variants (Function/Use/LiveBlock/Assert/Match/StructDecl/StructLiteral/FunctionLiteral)
  ROADMAP G6 cell flipped 🟡 → ✅. Every `Node` variant carries a
  `span: Span` field. Future work (more diagnostic surfacing,
  parser-error position threading) is incremental and tracked
  separately.
