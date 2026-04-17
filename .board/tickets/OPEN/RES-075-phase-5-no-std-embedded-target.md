---
id: RES-075
title: Phase 5 no_std embedded target
state: OPEN
priority: P3
goalpost: G16
created: 2026-04-17
owner: executor
---

## Summary
G16: prove Resilient programs can run on a Cortex-M class MCU. This
ticket carves out a `resilient-runtime` crate that builds under
`#![no_std]` against the `thumbv7em-none-eabihf` target. Step one is the
runtime — interpreting `.res` source on-device is out of scope here;
follow-up tickets (after RES-072 Cranelift backend matures) will AOT the
program and embed it.

## Acceptance criteria
- New crate `resilient-runtime` (workspace member) with
  `#![no_std]`, `extern crate alloc;`, and a `panic-halt` dependency.
- Crate exposes only the value types and core ops (Value enum, arithmetic,
  comparison) — NO file I/O, NO Z3, NO println.
- Builds cleanly under: `cargo build -p resilient-runtime --target thumbv7em-none-eabihf` (assuming the target is installed; ticket includes a `rustup target add` line in the README).
- A new `examples/embedded_blink/` skeleton (cortex-m + cortex-m-rt) that
  links against the runtime and computes `2 + 2` in main, asserting the
  result, then loops forever. Doesn't need to actually run on hardware —
  just needs to build.
- New CI job `embedded-build` invokes the cross-compile.
- Commit message: `RES-075: no_std runtime crate builds for thumbv7em`.

## Notes
- Allocator: pull in `embedded-alloc` and use `LlffHeap` — the
  tree-walking value model needs `Vec`/`String`.
- This ticket is intentionally narrow. Don't try to make the FULL
  interpreter `no_std` — it has Z3 + file I/O dependencies that don't
  belong on an MCU. Just the runtime/value layer.
- Will likely interact with RES-072 (JIT). If RES-072 isn't done yet,
  the embedded program just runs hand-written Rust calling into the
  runtime — that's enough to prove the link.

## Log
- 2026-04-17 created by manager
- 2026-04-17 acceptance criteria filled in by manager
