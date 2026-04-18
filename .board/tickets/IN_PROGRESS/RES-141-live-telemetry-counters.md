---
id: RES-141
title: Live-block telemetry counters exposed to the program
state: OPEN
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
