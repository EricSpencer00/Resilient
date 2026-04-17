---
id: RES-002
title: Add test harness with cargo test
state: OPEN
priority: P0
goalpost: G2
created: 2026-04-16
owner: executor
---

## Summary
The project has **zero tests**. No `#[test]`, no `tests/` directory,
no golden files. Before we can safely iterate on the language we need
a test harness so every subsequent ticket can add regression coverage.

## Acceptance criteria
- `cargo test` runs and produces at least 5 passing tests
- Tests cover the **lexer** (e.g. `fn foo(int x)` produces the expected
  token sequence — including the identifier-swallowing regression fixed
  in RES-001)
- Tests cover the **parser** (e.g. parsing `let x = 42;` produces the
  expected `Node` shape)
- Tests cover the **typechecker** happy path and one failure
- One integration test under `tests/` that parses `examples/hello.rs`
  to a non-empty `Program` (don't require running yet — that needs
  `println`, see RES-003)
- `cargo test` is clean — no warnings

## Notes
- Lexer/parser types in `main.rs` are currently private. To test them
  from integration tests under `tests/`, either make a thin `pub mod`
  surface or write tests as `#[cfg(test)] mod tests {}` inside the same
  file. Prefer in-file unit tests for the lexer and parser; integration
  tests under `tests/` for example runs.
- Do NOT fix the dummy-parameter limitation in this ticket — that's RES-004.

## Log
- 2026-04-16 created by session 0
