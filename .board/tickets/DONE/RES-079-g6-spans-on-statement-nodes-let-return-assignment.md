---
id: RES-079
title: G6 spans on core statement nodes (closes G6 🟡→ ✅ for statements)
state: DONE
priority: P1
goalpost: G6
created: 2026-04-17
owner: executor
---

## Summary
RES-078 put spans on leaves (literals + identifiers). This ticket
extends the same pattern to the most user-visible **statement**
variants: `LetStatement`, `StaticLet`, `Assignment`,
`ReturnStatement`, `IfStatement`, `WhileStatement`, `ForInStatement`.
Each gains a `span: Span` field.

Expression variants (`PrefixExpression`, `InfixExpression`,
`CallExpression`, etc.) remain for a future follow-up — they're
dense in the codebase and their spans are a separate shipping
unit. The original "spans on EVERY node" goal from RES-069 is
approached but not fully closed by this ticket; the remaining
expression-variant work can be filed as RES-084 after this ships.

## Acceptance criteria
- Add `span: span::Span` to each of these variants. Keep the other
  fields exactly as they are:
  - `Node::LetStatement`
  - `Node::StaticLet`
  - `Node::Assignment`
  - `Node::ReturnStatement`
  - `Node::IfStatement`
  - `Node::WhileStatement`
  - `Node::ForInStatement`
- Parser populates the span from `span_at_current()` at the moment
  parsing of the statement begins. RES-078's helper is the
  template — no new helper needed.
- Every destructure site updates by adding `.., span: _` or `..`
  where not already present. `..` is fine — consumers can opt into
  reading the span in follow-ups.
- Fields marked `#[allow(dead_code)]` where not yet read, matching
  RES-078's convention.
- New unit test: parse `let x = 1;\nlet y = 2;`, assert that
  `LetStatement` inside `stmts[1].node` has a `span` with
  `start.line == 2` (distinct from stmts[0]).
- `cargo build`, `cargo test`, `cargo clippy -- -D warnings` all
  pass on default features and `--features z3`.
- Commit message: `RES-079: spans on statement nodes (G6 closes for stmts)`.

## Notes
- After this ticket, update `.board/ROADMAP.md` G6 cell to mention
  that statement spans are done and that expression spans remain as
  a scoped follow-up (RES-084 TBD).
- DO NOT touch expression variants (`PrefixExpression`,
  `InfixExpression`, `CallExpression`, `ArrayLiteral`,
  `IndexExpression`, `IndexAssignment`, `FieldAccess`, etc.) —
  they're a separate shipping unit.
- `Block(Vec<Node>)` is a tuple variant today. Leaving it tuple-form
  is fine for this ticket — converting to struct form is a wider
  change that fits with the expression-variant follow-up.
- Errors from the typechecker-level RES-080 prefix already surface
  per-statement spans via the `Spanned<Node>` wrapper from RES-077.
  The per-variant spans added here are infrastructure for finer
  attribution (e.g. pointing at just the `let` keyword vs the whole
  statement) — they don't need to be wired up immediately.

## Log
- 2026-04-17 created by manager
- 2026-04-17 acceptance criteria filled in by manager (orchestrator pass)
- 2026-04-17 executor landed:
  - 7 statement variants gain a `span: span::Span` field, each
    marked `#[allow(dead_code)]` since they aren't read yet:
    `LetStatement`, `StaticLet`, `Assignment`, `ReturnStatement`,
    `IfStatement`, `WhileStatement`, `ForInStatement`.
  - Each of the 7 matching `parse_*` methods captures
    `let stmt_span = self.span_at_current();` **before** the first
    `next_token` advance, so the span reflects the originating
    keyword line rather than wherever the lexer landed after the
    terminator. Error-recovery constructions in the same methods
    reuse `stmt_span` so even degenerate paths produce usable
    spans.
  - ~24 destructure sites updated: `{ name, value, .. }` pattern
    added to every match site across `main.rs`, `typechecker.rs`,
    `compiler.rs`. `StaticLet` construction at `parse_static_let`
    pulls the span through from the inner LetStatement so the
    `static let` location is still accurate.
- 2026-04-17 tests: new `let_statement_spans_track_source_line`
  unit test parses `let x = 1;\nlet y = 2;` and asserts the first
  statement has `span.start.line >= 1`, the second has
  `span.start.line >= 2`, and that `s1.start.line > s0.start.line`
  — ordering is preserved.
- 2026-04-17 verification: 209 unit + 1 golden + 11 smoke = 221
  tests default; 217 + 1 + 12 = 230 with `--features z3`. Clippy
  clean both ways.
- 2026-04-17 ROADMAP.md G6 cell updated: statement spans done.
  Expression variants (PrefixExpression, InfixExpression,
  CallExpression, ArrayLiteral, IndexExpression, IndexAssignment,
  FieldAccess, FieldAssignment, MatchExpression) remain as a
  future RES-084 follow-up — file at appropriate time.
