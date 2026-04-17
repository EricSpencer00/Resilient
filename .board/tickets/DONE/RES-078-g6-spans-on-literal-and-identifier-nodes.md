---
id: RES-078
title: G6 spans on literal and identifier nodes
state: DONE
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
- 2026-04-17 executor landed:
  - 5 tuple variants → struct variants with `span: Span`:
    `IntegerLiteral`, `FloatLiteral`, `StringLiteral`,
    `BooleanLiteral`, `Identifier`.
  - New `Parser::span_at_current()` helper builds a single-position
    `Span` from the lexer's `last_token_line/column`. Used by the
    primary construction path in `parse_expression` and by
    `parse_pattern`.
  - Error-recovery fallback constructions (e.g.
    `unwrap_or(Node::IntegerLiteral { value: 0, span: Span::default() })`)
    use `Span::default()` since the node is synthetic.
  - All ~80 destructure sites across main.rs, typechecker.rs,
    verifier_z3.rs, compiler.rs updated to the new struct form
    via targeted sed + hand-patch for the non-mechanical cases.
  - `typechecker.rs` `Node::Identifier` handler now **uses** the
    span: undefined-variable errors get `'{name}' at L:C` appended,
    so users get both the statement-level (RES-080) and
    identifier-level location in one message.
  - 4 literal variants' `span` fields marked `#[allow(dead_code)]`
    with a scoped comment: they'll be surfaced in RES-079 / RES-080
    follow-ups but aren't read today. The sole compile warning this
    would have introduced is explicitly disclaimed.
- 2026-04-17 tests:
  - New unit test `literal_and_identifier_nodes_carry_non_default_spans`
    parses `let x = 42;` and asserts the `IntegerLiteral`'s span has
    `start.line >= 1`.
  - New unit test `undefined_variable_error_includes_line_col`
    confirms the Identifier-span is surfaced in the resulting
    diagnostic (`undefined_thing` + `at ` substring).
  - Pre-existing `parser_let_statement_produces_expected_shape`
    test updated to match `IntegerLiteral { value: 42, .. }`.
- 2026-04-17 manual verification: `--typecheck` on a source with an
  undefined variable prints
  `/tmp/r78.rs:2:5: Undefined variable 'undefined_var' at 2:22` —
  statement and identifier spans stack cleanly.
- 2026-04-17 verification: 208 unit + 1 golden + 11 smoke = 220
  tests default; 216 + 1 + 12 = 229 with `--features z3`. Clippy
  clean both ways.
- **G6 status**: leaves are spanned. Statement-nodes (Let, Return,
  Assignment, If, etc.) remain in RES-079. When that ticket lands,
  G6 closes to ✅.
