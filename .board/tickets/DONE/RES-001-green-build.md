---
id: RES-001
title: Green build — fix compile errors and identifier-swallowing lexer bug
state: DONE
priority: P0
goalpost: G1
created: 2026-04-16
owner: executor
---

## Summary
The crate had 17 compile errors and 2 warnings. Half-finished refactor
left `main.rs` reading `parser.errors` on a `Parser` that didn't carry
that field, plus a duplicate `Node` enum in `parser.rs`, a `println!`
missing a `{}`, and an identifier-swallowing bug in the hand-rolled
lexer that prevented `fn name(...)` from parsing at all.

## Acceptance criteria
- [x] `cargo build` succeeds with zero warnings
- [x] `cargo run -- examples/minimal.rs` reaches interpretation (fails
      only on the still-missing `println` builtin — separate ticket)

## Resolution
- Added `errors: Vec<String>` to `Parser`, initialized in `new()`,
  populated via new `record_error()` helper at the three existing
  parse-error sites.
- Fixed the lexer: identifier branch now early-returns (like the number
  branch) to avoid a trailing `self.read_char()` that was swallowing
  the character after every identifier.
- `parser.rs`: removed conflicting `Node` import; added `?` to 7
  `Result<Node, ParseError>` → `Node` mismatches; marked
  `#![allow(dead_code)]` until G6.
- `repl.rs`: dropped unused `Node` import; deleted bogus Result match on
  `parse_program()`; fixed `println!("fn add...", YELLOW)` missing
  placeholder.
- `main.rs`: `#[allow(dead_code)]` on the reference `start_repl` fn.

Landed in commit `cb5cb0f`.

## Log
- 2026-04-16 created and landed by session 0 (outside the loops)
