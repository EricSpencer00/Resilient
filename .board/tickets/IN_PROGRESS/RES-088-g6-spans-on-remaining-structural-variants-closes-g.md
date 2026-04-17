---
id: RES-088
title: G6 spans on remaining structural variants — CLOSES G6
state: OPEN
priority: P1
goalpost: G6
created: 2026-04-17
owner: executor
---

## Summary
The final G6 ticket. Adds `span: span::Span` to the eight remaining
struct variants:

- `Node::Function` (26 sites — the largest)
- `Node::Use` (10 sites)
- `Node::LiveBlock` (5)
- `Node::Assert` (5)
- `Node::StructLiteral` (5)
- `Node::FunctionLiteral` (5)
- `Node::Match` (5)
- `Node::StructDecl` (4)

All are already struct variants, so the change is purely additive
— no tuple-to-struct conversions needed. After this lands, every
`Node` variant carries a span and G6 flips ✅.

## Acceptance criteria
- Add `span: span::Span` to each of the eight variants listed
  above. Mark each `#[allow(dead_code)]` with a "consumed in
  follow-ups" comment matching RES-078/079/084/085/086/087.
- Parser populates the span at the appropriate keyword/token for
  each variant — typically the leading keyword (`fn`, `live`,
  `assert`, `struct`, `use`, `match`) captured BEFORE the first
  `next_token` advance.
- Every destructure site updated (~65 total) by adding `..` where
  not already present.
- Update existing tests that destructure these variants without
  `..` so they keep working.
- New unit test: parse a 2-statement source with two `fn`
  declarations on different lines, assert each `Function`'s span
  reflects its source line.
- `cargo build`, `cargo test`, `cargo clippy -- -D warnings` pass
  on all three feature configs (default, `--features z3`,
  `--features lsp`).
- ROADMAP.md G6 cell flipped from 🟡 to ✅ with the final
  status: "every Node variant carries a span; remaining work is
  surfacing them in more diagnostics, but the AST hardening goal
  is met."
- Commit message: `RES-088: spans on structural variants — G6 closes ✅`.

## Notes
- `Function` has 26 sites which is the bulk of this work. Many
  destructures already use `..` thanks to its 6 existing fields
  (name/parameters/body/requires/ensures/return_type). Adding a
  7th won't break those.
- For variants whose source location is harder to pin (Match arms,
  StructLiteral fields), capture the keyword span at entry to the
  parse method.
- `Node::Use` parser is in `parse_use_statement` around
  `main.rs:1262`. Capture span before consuming the `use` token.

## Log
- 2026-04-17 created by manager
- 2026-04-17 acceptance criteria filled in by manager (orchestrator pass)
