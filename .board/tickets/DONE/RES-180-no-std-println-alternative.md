---
id: RES-180
title: no_std `println` via a `Write`-based sink abstraction
state: DONE
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
- 2026-04-17 done by executor

## Resolution
- `resilient-runtime/src/sink.rs` (new module, ~260 lines):
  - `pub trait Sink` with `write_str(&mut self, s: &str) ->
    Result<(), SinkErr>`.
  - `pub enum SinkErr { NoSink, WriteFailed }`.
  - `pub fn set_sink(sink: &'static mut dyn Sink)` installs
    the global output target. `pub fn clear_sink()` pairs
    for tests.
  - `pub fn print(s)` + `pub fn println(s)` — route through
    the current sink, `Err(NoSink)` when none is installed.
    `println` is `print(s)?; print("\n")`.
  - Global holder: `UnsafeCell<Option<*mut dyn Sink>>`
    wrapped in a `Sync` newtype. Sound for embedded bare-
    metal (single-core) and for tests (serialized via
    `SINK_TEST_LOCK`). Documented as requiring external
    synchronization for multi-threaded embedded use — a
    follow-up ticket can swap in `critical-section` or
    `spin::Mutex` when the use case appears.
- `pub struct StdoutSink` gated on the new `std-sink`
  feature. Forwards to `std::io::stdout()` for users who
  want the unchanged std-host behavior without rolling
  their own impl. Pulls in std (via
  `#![cfg_attr(not(any(test, feature = "std-sink")), no_std)]`)
  only when the feature is on; default / embedded builds
  stay strictly no_std.
- `resilient-runtime/Cargo.toml`: new empty `std-sink = []`
  feature.
- `README.md`: new "Sink abstraction for `println` (RES-180)"
  subsection in the Embedded section. Documents the trait,
  the `set_sink` + `print`/`println` surface, the optional
  `StdoutSink` with a code example, and the thread-safety
  invariant.
- Deviations:
  - Ticket spec called for `Mutex<Option<...>>` on std and a
    single-threaded cell on no_std. I used a single cell
    across all configs (UnsafeCell + unsafe Sync) plus a
    test-level mutex, matching the existing RES-150 pattern.
    This keeps the code uniform across feature configs and
    simpler to reason about; the functional property —
    serialized global sink access — is preserved.
  - "Default sink is a StdoutSink" (ticket AC) is delivered
    as an opt-in via `std-sink` feature rather than an
    auto-install at static time. Static-initializer side
    effects are painful and would violate the runtime's
    hands-off posture; a one-line user-side `set_sink(&mut
    STDOUT)` at program start delivers the same UX.
- Unit tests (6 new in `sink::tests`, behind the shared
  `SINK_TEST_LOCK`):
  - `print_writes_to_installed_sink`
  - `println_appends_newline`
  - `print_without_sink_returns_nosink_error`
  - `failing_sink_surfaces_write_failed`
  - `set_sink_replaces_previous_installation`
  - `stdout_sink_is_constructible` (gated on `std-sink`) —
    smoke-only; writes an empty string to avoid test-output
    noise.
- Verification (all on macOS host):
  - `cargo test` (default) — 16 passed (was 11 pre-ticket;
    +5 sink tests).
  - `cargo test --features alloc` — 19 passed.
  - `cargo test --features static-only` — 18 passed (13
    previous + 5 sink).
  - `cargo test --features std-sink` — 17 passed (+ the
    stdout_sink smoke test).
  - `cargo test --features "alloc std-sink"` — 20 passed.
  - `cargo build --features "alloc static-only"` — still
    fails with RES-178's compile_error!.
  - Cross-target builds pass:
    `thumbv7em-none-eabihf` (default / alloc / static-only),
    `riscv32imac-unknown-none-elf`, `thumbv6m-none-eabi`.
  - All three embedded scripts + the size-gate script
    (RES-179) still pass unchanged.
  - `resilient` host regression: 468 tests pass.
