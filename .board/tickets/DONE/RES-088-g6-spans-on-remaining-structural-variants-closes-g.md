---
id: RES-088
title: G6 spans on remaining structural variants — CLOSES G6 ✅
state: DONE
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
- 2026-04-17 executor landed:
  - Added `span: span::Span` field (`#[allow(dead_code)]`) to all
    eight remaining struct variants: `Use`, `Function`, `LiveBlock`,
    `Assert`, `Match`, `StructDecl`, `StructLiteral`, `FunctionLiteral`.
  - `parse_function` captures `fn` keyword span before advance and
    threads it through every Function construction (3 sites: EOF
    fallback, error-recovery, normal). Other parsers fall back to
    `self.span_at_current()` for now.
  - Python-driven span injection across construction sites; manual
    fixes for sites the regex couldn't handle cleanly (mostly multi-
    line constructions with `Box::new` nested args).
  - ~70 destructure sites updated. Match-arm patterns that the
    injector wrecked (`Node::Foo { ..,\n  span: <expr> }` is invalid
    in pattern context) repaired via a follow-up Python sed pass to
    collapse them back to `, .. }`.
  - Typechecker `Node::Function` destructure at line 503 widened
    with `..`.
- 2026-04-17 tests:
  - New unit `function_declarations_carry_spans_per_source_line`:
    parses two functions on different lines, asserts each
    `Function`'s span reflects the originating line, and that line
    ordering is preserved.
- 2026-04-17 verification across three feature configs:
  - default: 215 unit + 1 golden + 11 smoke = 227 tests
  - `--features z3`: 222 + 1 + 12 = 235 tests
  - `--features lsp`: 217 + 1 + 11 = 229 tests
  All three `cargo clippy -- -D warnings` clean.
- ROADMAP G6 cell flipped 🟡 → ✅. **G6 closes.**
  Umbrella RES-069 also flipped to DONE with a summary log entry
  pointing at the RES-077..088 series.
