---
id: RES-147
title: `clock_ms()` builtin returns monotonic milliseconds
state: OPEN
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
