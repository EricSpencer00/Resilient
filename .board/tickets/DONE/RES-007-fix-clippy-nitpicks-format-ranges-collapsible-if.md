---
id: RES-007
title: Fix clippy nitpicks (format, ranges, collapsible if)
state: DONE
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

## Resolution
11 clippy warnings cleared without silencing. No new `#[allow(...)]`s.

- `main.rs:257` `ch.is_digit(10)` → `ch.is_ascii_digit()`
- `main.rs:600` redundant `let expr = ...; expr` → direct expression
- `main.rs:1080` `Float(f) if f == 0.0` → `Float(0.0)`
- `main.rs:1534` `for i in 1..args.len()` → `for arg in args.iter().skip(1)`
- `main.rs:1230` + `repl.rs:43` collapsible `if exists { if let Err ... }`
  → edition-2024 `if exists && let Err(err) = ...`
- `parser.rs:84` `len() > 0` → `!is_empty()`
- `parser.rs` 4× `format!("literal")` → `"literal".to_string()`

Verification:
```
$ cargo clippy -- -D warnings
Finished dev profile — no warnings
$ cargo test
11 unit + 2 integration, all passing
```

## Log
- 2026-04-16 created by session 0
