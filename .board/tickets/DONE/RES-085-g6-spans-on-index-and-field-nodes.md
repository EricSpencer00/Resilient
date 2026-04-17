---
id: RES-085
title: G6 spans on index and field nodes
state: DONE
priority: P1
goalpost: G6
created: 2026-04-17
owner: executor
---

## Summary
RES-084 spanned the three core expression variants (Prefix, Infix,
Call). This ticket extends the pattern to the four next-most-common
expression variants: `IndexExpression`, `IndexAssignment`,
`FieldAccess`, `FieldAssignment`. All four are struct variants
already, so the change is purely additive — no tuple-to-struct
conversions needed.

Remaining expression variants after this ticket — tuple variants
(`ArrayLiteral`, `TryExpression`, `Block`, `ExpressionStatement`)
and the large structural variants (`Match`, `StructLiteral`,
`FunctionLiteral`, `Function`, `LiveBlock`, `Assert`, `StructDecl`)
— are their own ticket when the pattern is needed there.

## Acceptance criteria
- Add `span: span::Span` field (marked `#[allow(dead_code)]` with
  "consumed in follow-ups" comment) to each of:
  - `Node::IndexExpression`
  - `Node::IndexAssignment`
  - `Node::FieldAccess`
  - `Node::FieldAssignment`
- Parser captures the span at the key token for each — typically
  the `[` for index ops, the `.` for field ops. Capture must happen
  BEFORE the corresponding `next_token` call, matching RES-079/084's
  convention.
- Every destructure site gets `..` added where not already present.
  Most sites already use `..` — only the cases that name all fields
  need updating.
- New unit test: parse `a[0]`, assert the `IndexExpression`'s span
  has `start.line >= 1`.
- `cargo build`, `cargo test`, `cargo clippy -- -D warnings` pass
  on default features, `--features z3`, AND `--features lsp`
  (three configs now — LSP landed in RES-074).
- Commit message: `RES-085: spans on index + field nodes (G6 partial)`.

## Notes
- Parser sites to update:
  - `parse_index_expression` around `main.rs:1855`
  - Index/field assignment at the `parse_maybe_index_assignment` /
    final destructure around `main.rs:770-780`
  - Field access in expression postfix around `main.rs:1815`
- Keep the diff tight — most destructures already have `..` thanks
  to the pattern established by RES-078/079/084.

## Log
- 2026-04-17 created by manager
- 2026-04-17 acceptance criteria filled in by manager (orchestrator pass)
- 2026-04-17 executor landed:
  - Added `span: span::Span` (marked `#[allow(dead_code)]`) to the
    four variants: `IndexExpression`, `IndexAssignment`,
    `FieldAccess`, `FieldAssignment`.
  - Parser `parse_index_expression` captures the `[` token's span
    before `next_token`, threads it through both the happy-path
    construction and the error-recovery return.
  - Parser `parse_field_access` captures the `.` token's span.
  - `parse_maybe_index_assignment`'s destructure pulls `span`
    through when converting an `IndexExpression`/`FieldAccess` LHS
    to an `IndexAssignment`/`FieldAssignment`. Keeps the assignment
    anchored at the originating `[` / `.` location.
  - ~10 destructure sites updated via targeted sed (`..` added).
- 2026-04-17 tests: new `index_and_field_expressions_carry_spans`
  unit test parses `let a = [1, 2]; a[0]; let b = a; b.len;` and
  asserts both the IndexExpression and FieldAccess nodes have
  non-default spans.
- 2026-04-17 verification across three feature configs:
  - default: 212 unit + 1 golden + 11 smoke = 224 tests
  - `--features z3`: 220 + 1 + 12 = 233
  - `--features lsp`: 215 + 1 + 11 = 227
  All three `cargo clippy -- -D warnings` clean.
- ROADMAP G6 cell updated. Remaining for full G6 closure: tuple
  variants (`ArrayLiteral`, `TryExpression`, `Block`,
  `ExpressionStatement`) which need tuple→struct conversion, and
  structural variants (`Match`, `StructLiteral`, `FunctionLiteral`,
  `Function`, `LiveBlock`, `Assert`, `StructDecl`).
