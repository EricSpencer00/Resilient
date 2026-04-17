---
id: RES-084
title: G6 spans on core expression nodes (Prefix/Infix/Call)
state: DONE
priority: P1
goalpost: G6
created: 2026-04-17
owner: executor
---

## Summary
RES-079 closed G6 for statement variants. This ticket does the same
for the three most user-visible **expression** variants —
`PrefixExpression`, `InfixExpression`, `CallExpression`. Those are
the ones that appear in typechecker / VM runtime errors most often
(e.g. type mismatch on `a + b`, arity mismatch on `f(x, y)`), so
pinning them with spans cashes in the diagnostic-quality work
immediately.

The remaining expression variants (`ArrayLiteral`, `IndexExpression`,
`IndexAssignment`, `FieldAccess`, `FieldAssignment`, `Match`,
`StructLiteral`, `FunctionLiteral`) are intentionally NOT in scope
here — they're a separate shipping unit once the pattern is proven
on the three core ones. This mirrors the RES-078/079 split.

## Acceptance criteria
- Add `span: span::Span` to each of:
  - `Node::PrefixExpression`
  - `Node::InfixExpression`
  - `Node::CallExpression`
- Each field annotated with `#[allow(dead_code)]` and a comment
  noting "consumed in follow-ups". Matches RES-078/079's convention.
- Parser populates the span from `span_at_current()` captured
  **before** any `next_token` call in the expression-construction
  site. For infix ops (built mid-expression in the Pratt loop),
  capture the span at the moment the operator token is observed.
- Every destructure site gets `..` added or span field acknowledged
  with `span: _`. About 40-50 sites across `main.rs`,
  `typechecker.rs`, `verifier_z3.rs`, `compiler.rs`.
- New unit test: parse `1 + 2`, assert the resulting
  `InfixExpression`'s span has `start.line >= 1`.
- `cargo build`, `cargo test`, `cargo clippy -- -D warnings` all
  pass on default features and `--features z3`.
- Commit message: `RES-084: spans on core expression nodes (G6 partial closes)`.

## Notes
- The three variants I chose cover the most common diagnostic
  failure modes. Remaining expression variants can share a follow-up
  ticket once the pattern here is proven.
- Parser's `parse_expression` is where all three are built. The
  prefix case captures a span at entry; infix captures at the
  operator token; call captures at the `(` token.
- After this lands, G6 in ROADMAP.md can drop the "core expression
  variants" qualifier from its status line.

## Log
- 2026-04-17 created by manager
- 2026-04-17 acceptance criteria filled in by manager (orchestrator pass)
- 2026-04-17 executor landed:
  - `Node::PrefixExpression`, `Node::InfixExpression`,
    `Node::CallExpression` each gain a `span: span::Span` field
    marked `#[allow(dead_code)]`. Field comment notes "consumed in
    follow-ups" (typechecker arithmetic-mismatch, arity-mismatch).
  - Parser captures a span at the moment of each expression's key
    token: prefix captures at the `!` / `-` token, infix at the
    operator token between lhs and rhs, call at the `(` token.
    All three capture **before** the corresponding `next_token`,
    matching RES-079's convention.
  - ~21 destructure sites across `main.rs`, `typechecker.rs`,
    `verifier_z3.rs`, `compiler.rs` updated via targeted sed to
    add `..` where missing.
  - verifier_z3 tests had ~12 InfixExpression constructors that
    needed `span: crate::span::Span::default()`. Fixed via a
    Python-driven injection pass + three manual fixes for
    nested-inside-nested cases the script missed.
- 2026-04-17 tests:
  - `infix_expression_carries_operator_span`: parses `1 + 2;` and
    asserts the InfixExpression's span has `start.line >= 1`.
  - `prefix_and_call_expressions_carry_spans`: parses a
    three-statement source and checks both `!true` and `f()` have
    populated spans.
- 2026-04-17 verification: 211 unit + 1 golden + 11 smoke = 223
  tests default; 219 + 1 + 12 = 232 with `--features z3`. Clippy
  clean both ways.
- ROADMAP G6 cell updated to call out RES-084; remaining
  secondary-expression variants (ArrayLiteral, IndexExpression,
  IndexAssignment, FieldAccess, FieldAssignment, Match,
  StructLiteral, FunctionLiteral, TryExpression) tracked as a
  future follow-up ticket.
