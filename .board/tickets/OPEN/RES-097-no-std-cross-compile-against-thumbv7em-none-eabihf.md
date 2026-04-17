---
id: RES-097
title: resilient-runtime cross-compiles for thumbv7em-none-eabihf
state: OPEN
priority: P3
goalpost: G16
created: 2026-04-17
owner: executor
---

## Summary
RES-075 Phase A landed `resilient-runtime/` with `#![no_std]` lib
+ Value enum + core ops, but only verified the HOST build. This
ticket proves the same crate cross-compiles against a real
embedded target (`thumbv7em-none-eabihf`, the Cortex-M4F class
MCU).

This is the first ticket where embedded toolchain matters: the
target needs to be installed via `rustup target add` before
cargo can drive the build.

## Acceptance criteria
- `cd resilient-runtime && cargo build --target thumbv7em-none-eabihf`
  succeeds when the target is installed (`rustup target list --installed`
  shows `thumbv7em-none-eabihf`).
- The existing 7 unit tests still pass on host (`cargo test`).
- Top-level `README.md`'s "Embedded runtime" section gains a
  "verified cross-compile" subsection with the exact commands.
- `resilient-runtime/Cargo.toml` does NOT change in a way that
  breaks the host build — features may be added but the default
  feature set must continue to build on host.
- New `resilient-runtime/.cargo/config.toml` (or equivalent) MAY
  be added if it's needed to default the target — preferred is
  to leave the default at host and require `--target` for cross.
- Commit message: `RES-097: resilient-runtime cross-compiles for thumbv7em-none-eabihf`.

## Notes
- Pre-flight: `rustup target add thumbv7em-none-eabihf` (the
  ticket's success is contingent on this being done locally; CI
  setup is RES-099).
- The `Value::div` implementation uses `i64::wrapping_*` and the
  raw `/` operator — both should be no_std-clean. Verify by
  trying the cross-compile.
- If the cross-compile turns up missing-symbol errors (e.g.
  `__udivdi3` for 64-bit divide on Cortex-M), document them in
  the ticket log and either (a) add `compiler_builtins` as a dep
  or (b) gate the failing op behind a feature flag for host-only
  use. Pick whichever is less invasive.
- After this ticket: RES-098 adds `embedded-alloc` so Vec/String
  variants can join `Value`.

## Log
- 2026-04-17 created by manager
- 2026-04-17 acceptance criteria filled in by manager (orchestrator pass)
