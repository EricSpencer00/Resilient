---
id: RES-070
title: G6 delete unused parser.rs module
state: OPEN
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
