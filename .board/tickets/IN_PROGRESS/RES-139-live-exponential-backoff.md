---
id: RES-139
title: Optional exponential backoff between live-block retries
state: OPEN
priority: P3
goalpost: G10
created: 2026-04-17
owner: executor
---

## Summary
Tight retry loops on flaky hardware can hammer the failing
component (I2C bus, sensor, etc.). Offer an opt-in backoff policy:
`live backoff(base_ms=1, factor=2, max_ms=100) { ... }` sleeps
between retries on a capped exponential curve.

## Acceptance criteria
- Syntax: `live backoff(base_ms = N, factor = K, max_ms = M) {
  ... }` — each kwarg optional with defaults 1 / 2 / 100.
- Runtime: after a failed attempt, sleep
  `min(max_ms, base_ms * factor^retries)` then retry. Sleep
  implementation is `std::thread::sleep` on std, `cortex_m::asm::delay`
  when targeting the no_std embedded runtime (RES-098's alloc
  variants).
- Plain `live { ... }` without the `backoff(...)` prefix preserves
  current zero-sleep semantics.
- Unit tests (std): three-failure sequence, verify wall-clock
  elapsed ≥ sum of computed sleeps.
- Commit message: `RES-139: live-block exponential backoff`.

## Notes
- Do not offer unbounded `factor`. Cap at 10 and error on exceed —
  prevents accidental `factor=1e9` runaway.
- The no_std delay implementation requires a clock abstraction we
  don't have yet; gate that behind a runtime feature and leave
  the concrete wiring for a follow-up.

## Log
- 2026-04-17 created by manager
