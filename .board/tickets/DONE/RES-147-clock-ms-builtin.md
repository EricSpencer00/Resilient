---
id: RES-147
title: `clock_ms()` builtin returns monotonic milliseconds
state: DONE
priority: P3
goalpost: G11
created: 2026-04-17
owner: executor
---

## Summary
Benchmarks in-language, simple rate-limiting, and anything that
asks "how long has this been running" want a monotonic clock. Wall
clock is a worse default (jumps back on NTP sync) so we expose
only a monotonic variant.

## Acceptance criteria
- `clock_ms() -> Int` returns milliseconds since an unspecified
  process-lifetime epoch. Monotonic: two sequential calls never
  return a decreasing pair.
- Implementation (std): `std::time::Instant` captured on first
  call; subsequent calls return `(Instant::now() - epoch).as_millis()`
  clamped to i64 range.
- no_std: not registered (embedded has no stdlib clock yet;
  follow-up ticket under G16 would wire `embedded-time`).
- Unit test: sleep 10ms, assert difference is ≥ 9ms and ≤ 50ms.
  Uses `std::thread::sleep`.
- Commit message: `RES-147: clock_ms() monotonic builtin`.

## Notes
- The first call is cheap; the epoch is captured into a
  `std::sync::OnceLock`.
- Explicitly don't expose seconds / minutes helpers — the language
  doesn't need a time module yet; `clock_ms()` / 1000 works.

## Log
- 2026-04-17 created by manager
- 2026-04-17 claimed by executor
- 2026-04-17 done by executor

## Resolution
- `resilient/src/main.rs`:
  - New `static CLOCK_EPOCH: std::sync::OnceLock<Instant>` —
    captured lazily on the first call.
  - New `builtin_clock_ms(args)` — zero-arg. Reads the epoch via
    `get_or_init(Instant::now)`, samples `Instant::now()`, and
    returns `(now - epoch).as_millis()` clamped to `i64::MAX`
    on overflow. Monotonicity is guaranteed by `Instant` on
    every supported platform; `duration_since` is saturating,
    so even exotic scheduler drift can't go backwards. The
    epoch is unobservable except through delta reads — the
    ticket's "unspecified process-lifetime epoch".
  - Registered in the `BUILTINS` table.
- `resilient/src/typechecker.rs`: `clock_ms` registered as
  `fn() -> Int`.
- Deviations: none. std-only; no_std runtime deliberately
  doesn't get it (ticket says "follow-up ticket under G16
  would wire `embedded-time`").
- Unit tests (3 new):
  - `clock_ms_advances_after_sleep` — the ticket's exact
    recipe: sleep 10ms, assert `9 <= delta <= 50` (written as
    a `RangeInclusive::contains` per clippy).
  - `clock_ms_never_goes_backwards` — 10 rapid calls produce
    a non-decreasing sequence. Documents the monotonicity
    invariant at the builtin boundary.
  - `clock_ms_rejects_arguments` — zero-arity check.
- Verification:
  - `cargo test --locked` — 363 passed (was 360 before RES-147)
  - `cargo test --locked --features logos-lexer` — 364 passed
  - `cargo clippy --locked --features logos-lexer,z3 --tests
    -- -D warnings` — clean
