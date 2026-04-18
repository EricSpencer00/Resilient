---
id: RES-141
title: Live-block telemetry counters exposed to the program
state: DONE
priority: P3
goalpost: G10
created: 2026-04-17
owner: executor
---

## Summary
For field diagnostics we need the runtime to tell us how often
live blocks retried during a run. Add two builtins:
`live_total_retries() -> Int` (process-wide counter) and
`live_total_exhaustions() -> Int` (times a live block gave up).

## Acceptance criteria
- Two new builtins reading from process-global atomic counters.
  Reads are cheap (relaxed ordering is fine for diagnostics).
- Counters reset per program run; no attempt at persistence.
- `examples/telemetry_demo.rs` + `.expected.txt` exercises both
  builtins after a deliberate sequence of failures.
- Unit tests confirming the counters advance as expected across
  nested blocks (RES-140 semantics).
- Commit message: `RES-141: live-block telemetry builtins`.

## Notes
- In the no_std runtime the atomics are still fine — Cortex-M
  supports `AtomicU32` natively. Use `u32` (not u64 — thumbv7 only
  has 32-bit atomics).
- Don't add per-span counters yet — that's a follow-up for a
  different ticket if it turns out we need site-level data.

## Log
- 2026-04-17 created by manager
- 2026-04-17 claimed by executor
- 2026-04-17 done by executor

## Resolution
- `resilient/src/main.rs`:
  - Added process-global atomics
    `LIVE_TOTAL_RETRIES: AtomicU32` and
    `LIVE_TOTAL_EXHAUSTIONS: AtomicU32` (u32 for
    Cortex-M/thumbv7 compatibility per ticket notes).
  - New builtins `builtin_live_total_retries` /
    `builtin_live_total_exhaustions` — zero-arg, read
    counters with `Ordering::Relaxed` (diagnostics only),
    return `Value::Int(n as i64)`.
  - Wired both into the `BUILTINS` table.
  - `eval_live_block` bumps `LIVE_TOTAL_RETRIES` only on
    actual retry branches (`retry_count < MAX_RETRIES`),
    and bumps `LIVE_TOTAL_EXHAUSTIONS` once on the
    give-up branch before returning `Err`. This keeps the
    semantics crisp: retries count transitions that led
    to another attempt; exhaustions count blocks that
    truly gave up.
- `resilient/src/typechecker.rs`: registered
  `live_total_retries` and `live_total_exhaustions` as
  `fn() -> Int` in the prelude env.
- Unit tests (in `main.rs` test module):
  - `live_total_retries_zero_arity` — confirms arity
    checking and initial `Int(0)` read.
  - `live_total_counters_advance_on_retries_and_exhaustions`
    exercises nested blocks (RES-140 semantics) with a
    deliberate 2-fail-then-succeed inner block plus an
    exhausting outer block, asserting counter *deltas*
    with `>=` lower bounds so the test is robust to
    parallel test pollution of the process-wide atomics.
- `resilient/examples/telemetry_demo.rs` +
  `telemetry_demo.expected.txt`: program reads both
  counters before and after a `live` block that fails
  twice then succeeds. Golden output shows
  `retries=2, exhaustions=0` after, matching the spec.
- Deviations: none from acceptance criteria. Counters are
  `u32` as the notes suggested.
- Verification:
  - `cargo test --locked` — 317 passed.
  - `cargo test --locked --features logos-lexer` — 318 passed.
  - `cargo clippy --locked --features logos-lexer,z3 --tests -- -D warnings` — clean.
  - Example: `cargo run --example telemetry_demo` matches
    `telemetry_demo.expected.txt`.
