---
id: RES-085
title: G6 spans on index and field nodes
state: OPEN
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
