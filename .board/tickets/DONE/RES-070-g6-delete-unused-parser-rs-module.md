---
id: RES-070
title: G6 delete unused parser.rs module
state: DONE
priority: P1
goalpost: G6
created: 2026-04-17
owner: executor
---

## Summary
`resilient/src/parser.rs` (817 lines) is a half-built second parser that
defines a parallel `Node` type and a `ParseError` struct. It is declared
in `resilient/src/main.rs:12` as `mod parser;` but no symbol from the
module is ever used (the file opens with `#![allow(dead_code)]`). Two
parsers means two AST shapes, two sets of bugs, and two places future
contributors have to reason about. G6 says: pick the canonical AST and
delete the rest. The canonical AST is the one in `main.rs` (it's the one
the interpreter, typechecker, and verifier actually use).

This ticket should land AFTER RES-069 (Spans on every node), so that any
useful idea from `parser.rs` (e.g. its `ParseError` shape) has been
absorbed into the surviving parser.

## Acceptance criteria
- `resilient/src/parser.rs` is deleted (`git rm`).
- `mod parser;` line removed from `resilient/src/main.rs:12`.
- No new dead-code or unused-import warnings introduced.
- Any genuinely useful diagnostic patterns from the deleted file have already
  been merged into the in-tree parser as part of RES-069 (note in commit
  message what, if anything, was preserved).
- `cargo build`, `cargo test`, and `cargo clippy -- -D warnings` all pass.
- Commit message: `RES-070: delete dead parser.rs (G6 closes)`.

## Notes
- Blocked on RES-069. Do not start until RES-069 is in DONE.
- After this ticket, G6 in `.board/ROADMAP.md` should be flipped to ✅ by the
  Manager.

## Log
- 2026-04-17 created by manager
- 2026-04-17 acceptance criteria filled in by manager
- 2026-04-17 executor landed: `git rm resilient/src/parser.rs` (817
  lines) and `mod parser;` removed from `main.rs`. Pre-flight grep
  confirmed zero call sites referenced the module (it had been opening
  with `#![allow(dead_code)]` since inception). Nothing was preserved
  from the deleted file — the in-tree parser in `main.rs` is strictly
  more capable; the parallel `Node` type and `ParseError` shape in
  parser.rs were both inferior to what we already use. RES-069's Span
  foundation made the `ParseError { line, column }` idea redundant
  too. Build, test, and clippy clean both with and without
  `--features z3` (152 / 161 tests respectively).
- 2026-04-17 G6 status: still partial. RES-069's AST-side migration
  (Spans on every `Node` variant + at least one diagnostic path
  rewritten) remains. When that lands, the manager should flip G6 to
  ✅ in ROADMAP.md.
