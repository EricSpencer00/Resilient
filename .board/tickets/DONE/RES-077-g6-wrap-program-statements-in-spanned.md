---
id: RES-077
title: G6 wrap Program statements in Spanned
state: DONE
priority: P1
goalpost: G6
created: 2026-04-17
owner: executor
---

## Summary
RES-069's foundation pass shipped `span.rs` (Pos/Span/Spanned) and a
lexer helper (`next_token_with_span`) but did NOT migrate the AST.
This ticket is the smallest meaningful next step: every statement at
the **top level** of a `Program` carries a `Span`. That's what the
upcoming LSP work (RES-074) actually needs to draw red squigglies on
the right line — it doesn't need spans on every sub-expression yet.

The strategy is additive, not breaking: keep the existing `Node`
enum unchanged, but change `Node::Program(Vec<Node>)` to
`Node::Program(Vec<Spanned<Node>>)`. Every match site that destructures
`Program` (the interpreter, the typechecker, anything in tests) gets a
mechanical `s.node` deref. Sub-expressions still have no spans —
that's RES-078 / RES-079.

## Acceptance criteria
- `Node::Program` variant changed from `Vec<Node>` to `Vec<Spanned<Node>>`.
- Parser populates each statement's span: `start = the lexer's
  last_token_line/column at the moment parse_statement was entered`,
  `end = same at the moment it returned`. (Use the existing
  `next_token_with_span` helper or its equivalent.)
- Every match site that destructures `Node::Program` is updated. The
  fallout is mechanical — typically `for s in stmts` becomes
  `for s in stmts { let stmt = &s.node; ... }`. Build until clean.
- Existing tests that construct a `Node::Program` directly (search for
  `Node::Program(`) get updated to wrap their statements with
  `Spanned::new(stmt, Span::default())`.
- New unit test in `main.rs` `mod tests`: parse `let x = 1; let y = 2;`,
  assert that `Program` has 2 spanned statements and that the second
  statement's `start.line == 1` and `start.column > first.start.column`
  (or `start.line == 2` if the test uses `\n` separator). Either way:
  prove spans are non-default and ordered.
- `cargo build`, `cargo test`, `cargo clippy -- -D warnings` all pass
  on default features and `--features z3`.
- Commit message: `RES-077: Program statements carry Span (G6 partial)`.

## Notes
- `Node::Program` is at `resilient/src/main.rs:495`.
- Top-level destructure sites:
  - `eval_program` at `:2972` (`statements: &[Node]` becomes `&[Spanned<Node>]`)
  - `typechecker::check_program` at `:402`
  - `imports::expand_uses` in `resilient/src/imports.rs` (the `Vec` of stmts and the matching there)
  - test helpers like `parse(&str) -> (Node, Vec<String>)` callers
- Be careful with `imports::expand_uses` — it currently does
  `stmts.drain(..)` on the `Vec`. After the change, it'll be draining
  `Spanned<Node>` values; the `Node::Use { path }` destructure happens
  on `s.node`, not `s`.
- DO NOT touch `Node::Function`, `Node::Block`, expressions, etc. in
  this ticket. Their span migration is RES-078 / RES-079. Keep the
  diff under ~250 lines.

## Log
- 2026-04-17 created by manager
- 2026-04-17 acceptance criteria filled in by manager (orchestrator pass)
- 2026-04-17 executor landed:
  - `Node::Program(Vec<Node>)` → `Node::Program(Vec<span::Spanned<Node>>)`.
  - `Parser::parse_program` snapshots `lexer.last_token_line/column`
    before and after each `parse_statement` to populate the per-stmt
    `Span`. Offsets remain 0 (we don't yet thread them); line/column
    are the actionable parts.
  - `Interpreter::eval_program` signature `&[Node]` → `&[Spanned<Node>]`,
    derefs via `.node` for both the function-hoist pre-pass and the
    main eval loop.
  - `typechecker::check_program` mirror update — both the contract-
    table pre-pass and the per-stmt `check_node` loop deref `.node`.
  - `imports::expand_uses` rewritten to thread `Spanned<Node>` through
    the splice — the `Node::Use` destructure happens on `stmt.node`
    and recursive imports preserve span on the inserted statements.
  - 6 in-tree test sites that did `match &stmts[0] { ... }` updated
    to `match &stmts[0].node { ... }` via a single targeted sed.
- 2026-04-17 acceptance test: `program_statements_carry_non_default_spans`
  parses a two-statement source, asserts both `Spanned`s have
  `start.line >= 1`, and asserts the second statement's start line is
  strictly later than the first's. Pre-existing parser shape test
  (`parser_let_statement_produces_expected_shape`) updated to reach
  through `.node`.
- 2026-04-17 verification: 166 unit + 1 golden + 6 smoke = 173 tests
  default. With `--features z3`: 174 + 1 + 7 = 182 tests. Clippy
  clean both ways. Diff is ~80 lines net (well under the 250-line
  guidance) — the limit on scope held.
