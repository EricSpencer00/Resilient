---
id: RES-180
title: no_std `println` via a `Write`-based sink abstraction
state: IN_PROGRESS
priority: P3
goalpost: G16
created: 2026-04-17
owner: executor
---

## Summary
`println` on std writes to stdout. On embedded there is no
stdout — users have UART, semihosting, or a ring buffer, and each
project picks one. Abstract the write path behind a trait so the
runtime doesn't bake in a sink.

## Acceptance criteria
- `resilient-runtime` exports a trait `Sink` with one method
  `write_str(&mut self, s: &str) -> Result<(), SinkErr>`.
- `println` + `print` route through a
  `static OUT: Mutex<Option<&'static mut dyn Sink>>` (std) or
  a single-threaded static cell (no_std).
- A helper `set_sink(sink: &'static mut dyn Sink)` installs the
  sink at program start.
- On std, default sink is a `StdoutSink` — unchanged behavior.
- Unit tests (std) with a memory-backed Sink asserting output is
  captured there.
- Commit message: `RES-180: println via Sink abstraction`.

## Notes
- Mutex on std; on no_std use a `critical-section` cell via the
  `critical-section` crate if added cost is acceptable, or a
  raw unsync cell if single-threaded is safe to assume
  (usual for bare-metal).
- Don't hard-code a UART driver — that's the application's
  concern. We just provide the plumbing.

## Log
- 2026-04-17 created by manager
- 2026-04-17 claimed by executor
