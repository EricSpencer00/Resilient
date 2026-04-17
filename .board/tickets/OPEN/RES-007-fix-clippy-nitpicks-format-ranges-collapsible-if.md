---
id: RES-007
title: Fix clippy nitpicks (format, ranges, collapsible if)
state: OPEN
priority: P3
goalpost: G1
created: 2026-04-16
owner: executor
---

## Summary
`cargo clippy` currently reports ~11 warnings (useless `format!`,
needless_range_loop, collapsible_if, char::is_digit with literal
radix 10, returning result of let binding, redundant guard, etc.).
None are blocking, but clearing them now keeps the executor loop's
verification signal clean.

## Acceptance criteria
- `cargo clippy -- -D warnings` from inside `resilient/` exits 0
- No functional behavior changes (output of examples unchanged)
- `cargo test` still passes
- The `#[allow(...)]` count does not grow — fix, don't silence

## Notes
- Run `cargo clippy 2>&1 | head -200` to see the current list.
- Rust edition is 2024 — `collapsible_if` has an `&& let` form that's
  edition-2024-native.
- Hot spots: `src/main.rs` argument parsing loop (`needless_range_loop`),
  `src/repl.rs` format strings, `src/main.rs:253` `is_digit(10)`.

## Log
- 2026-04-16 created by session 0
