---
id: RES-078
title: G6 spans on literal and identifier nodes
state: OPEN
priority: P1
goalpost: G6
created: 2026-04-17
owner: executor
---

## Summary
RES-077 put spans on every top-level statement. RES-080 uses those
spans in typechecker diagnostics. This ticket does the per-leaf
work: the five leaf AST nodes (`IntegerLiteral`, `FloatLiteral`,
`StringLiteral`, `BooleanLiteral`, `Identifier`) gain a `Span` field.
That finishes the leaves; statement nodes (Let/Return/Assignment/
If/etc.) are RES-079.

Strategy: change the tuple variants to struct variants with a
`span: Span` field. Every constructor supplies a span (use the
lexer's `last_token_line/column` at parse time; `Span::default()`
for test constructors); every destructure uses `{ value, .. }` or
similar to ignore the span. 88 references across the codebase —
mostly mechanical.

## Acceptance criteria
- `Node::IntegerLiteral(i64)` → `Node::IntegerLiteral { value: i64, span: Span }`.
- `Node::FloatLiteral(f64)` → `Node::FloatLiteral { value: f64, span: Span }`.
- `Node::StringLiteral(String)` → `Node::StringLiteral { value: String, span: Span }`.
- `Node::BooleanLiteral(bool)` → `Node::BooleanLiteral { value: bool, span: Span }`.
- `Node::Identifier(String)` → `Node::Identifier { name: String, span: Span }`.
- Parser construction sites populate the span from
  `span::Pos::new(self.lexer.last_token_line, self.lexer.last_token_column, 0)`
  for both start and end (leaf tokens span a single position; RES-079
  can widen this to cover the full lexeme width when it threads
  `next_token_with_span`).
- Every destructure site updates to the new struct form. The `..` ignore
  keeps the diff from touching irrelevant code.
- Test constructors that build literals directly use `Span::default()`.
- New unit test: parse `let x = 42;`; assert that the `value` inside
  the `LetStatement`'s `IntegerLiteral` has a non-default span whose
  `start.line == 1`.
- `cargo build`, `cargo test`, `cargo clippy -- -D warnings` all pass
  on default features and `--features z3`.
- Commit message: `RES-078: spans on literal + identifier nodes (G6 partial)`.

## Notes
- `Node` is at `resilient/src/main.rs:494`. Span type comes from the
  `span` module at `resilient/src/span.rs` (imported into main.rs
  already as `use span::{Pos, Span, Spanned};`).
- Files that destructure these variants: `main.rs` (parser +
  interpreter + tests), `typechecker.rs`, `verifier_z3.rs`,
  `compiler.rs`. The `imports.rs` module and `vm.rs` don't touch
  any of these variants directly — they work with already-compiled
  state.
- Keep the diff tight: use `..` aggressively to skip the span in
  every destructure. Only one or two diagnostic paths actually need
  the span value today.
- **No Bail-out into RES-079** scope: statement-node spans (Let,
  Return, Assignment, If, etc.) remain RES-079. Leave them as-is.

## Log
- 2026-04-17 created by manager
- 2026-04-17 acceptance criteria filled in by manager (orchestrator pass)
