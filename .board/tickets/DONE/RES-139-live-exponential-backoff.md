---
id: RES-139
title: Optional exponential backoff between live-block retries
state: DONE
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
- 2026-04-17 claimed by executor
- 2026-04-17 done by executor

## Resolution

Files changed:
- `resilient/src/main.rs`
  - New `struct BackoffConfig { base_ms, factor, max_ms }` with
    `default_ticket()` (1 / 2 / 100) and a `delay_ms(retries)`
    method that returns `min(max_ms, base_ms * factor^retries)`.
    Uses `saturating_pow` / `saturating_mul` so aggressive
    configs can't overflow `u64`.
  - `Node::LiveBlock` gains `backoff: Option<BackoffConfig>`.
    `None` = existing zero-sleep behaviour (all existing
    `live { ... }` blocks keep working unchanged).
  - `parse_live_block` detects the optional `backoff(...)`
    prefix by matching `Identifier("backoff")` after the `live`
    token — context-sensitive, no new reserved word.
  - New `parse_backoff_kwargs` — parses `(base_ms=N, factor=K,
    max_ms=M)` with each kwarg optional. Rejects `factor > 10`
    with the ticket's rationale message; non-negative integer
    literals required; unknown kwargs rejected.
  - `eval_live_block` takes `backoff: Option<&BackoffConfig>`
    and, after a failed attempt (before the env snapshot
    restore), sleeps via `std::thread::sleep(Duration::from_millis(
    delay))` where `delay = cfg.delay_ms(retry_count - 1)`. Zero
    `delay` skips the syscall.
  - Seven new unit tests: `delay_ms` cap / saturation math,
    kwargs-populated / defaults-applied / factor-over-10
    rejection / plain-live-stays-None, and
    `backoff_sleeps_between_retries` which asserts wall-clock
    elapsed ≥ 60 ms on a 20 / 40 sleep sequence (generous
    lower bound — `std::thread::sleep` only lower-bounds the
    duration).

No AST churn beyond the `LiveBlock` field: the three
constructor sites all live in `main.rs`'s parser, all updated.

Deviation: the no_std / embedded-runtime sleep hook is left
for a follow-up as the ticket's notes explicitly permit —
`resilient-runtime` has no clock abstraction today; a proper
`cortex_m::asm::delay` wiring needs one. The `std` path
(which the test suite exercises) is the full delivery.

Verification:
- `cargo build --locked` — clean.
- `cargo test --locked` — 313 unit (+7 new) + 3 dump-tokens +
  12 examples-smoke + 1 golden pass.
- `cargo test --locked --features logos-lexer` — 314 unit pass.
- `cargo clippy --locked --features logos-lexer,z3,jit --tests -- -D warnings`
  — clean.
- Manual: `live backoff(base_ms=5, factor=3, max_ms=50) { ... }`
  with 2 forced failures runs to success and prints the body's
  output; the sleep between retries is observable on wall clock
  but the default non-backoff `live { ... }` still runs at
  full speed.
