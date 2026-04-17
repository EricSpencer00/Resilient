---
id: RES-075
title: Phase 5 no_std embedded target
state: DONE
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
**Phase A scope (this ticket)**: foundation only — a separate
`resilient-runtime` crate that builds with `#![no_std]` on the
HOST (no cross-compile yet). Embedded-target build + actual
cortex-m examples land in RES-100+ follow-ups. Mirrors how
RES-072 + RES-074 landed scaffolding before real implementation.

- New sibling directory `resilient-runtime/` (not a workspace
  member — keeps the existing single-crate `resilient/` layout
  untouched). Has its own `Cargo.toml`.
- `resilient-runtime/src/lib.rs`:
  - `#![no_std]`
  - `extern crate alloc;`
  - `pub enum Value` with `Int(i64)`, `Bool(bool)` variants for
    starters (Float/String/Array follow once they're proven on
    the embedded target).
  - `pub fn add(a: Value, b: Value) -> Result<Value, RuntimeError>`
    plus matching sub/mul. Wrap-on-overflow semantics matching
    the bytecode VM.
  - `pub enum RuntimeError { TypeMismatch(&'static str), DivideByZero }`.
- 4-6 unit tests in `resilient-runtime/src/lib.rs` `mod tests`
  covering: int add round-trip, type-mismatch on mixed Int+Bool,
  div-by-zero, bool round-trip.
- `cargo build` and `cargo test` run from `resilient-runtime/`
  succeed on the developer's host (no embedded target required
  for Phase A — the `#![no_std]` annotation is what matters).
- Top-level `README.md` gains an "Embedded runtime" section
  documenting how to install `thumbv7em-none-eabihf` and run a
  manual cross-compile (which lands in a follow-up ticket).
- The existing `resilient/` crate is untouched. Default build,
  z3 build, lsp build, jit build all continue to pass.
- Commit message: `RES-075: resilient-runtime crate (Phase A — no_std lib + value types)`.

**Out of scope (split into follow-ups):**
- Cross-compile against `thumbv7em-none-eabihf` — RES-100.
- `embedded-alloc` integration for Vec/String support — RES-101.
- cortex-m + cortex-m-rt example program — RES-102.
- CI job that runs the cross-compile — RES-103.
- AOT lowering of Resilient programs to embed in firmware — depends
  on RES-072's JIT track maturing (RES-098+).

## Notes
- Why a sibling crate and not a workspace member: keeps the change
  blast radius small. The `resilient/` build, tests, clippy, and
  feature configs are completely untouched. A future ticket can
  promote both to a workspace when there's a real reason
  (shared profile config, cross-crate testing).
- Phase A's value types are deliberately simpler than `resilient/
  src/main.rs::Value` — that one carries `Box<Node>` for closures,
  which transitively pulls in alloc + Vec. Embedded targets need
  to opt into alloc explicitly (RES-101); Phase A stays alloc-free
  to prove the no_std boundary first.

## Log
- 2026-04-17 created by manager
- 2026-04-17 acceptance criteria filled in by manager
- 2026-04-17 manager rescope: Phase A only (sibling no_std lib +
  Value enum + core ops). Cross-compile, embedded-alloc, cortex-m
  example all split into RES-100..103.
- 2026-04-17 executor landed Phase A:
  - New sibling directory `resilient-runtime/` with its own
    `Cargo.toml` (NOT a workspace member — keeps the existing
    `resilient/` crate untouched).
  - `resilient-runtime/src/lib.rs`:
    - `#![cfg_attr(not(test), no_std)]` so the libtest harness
      can use std but production builds stay no_std.
    - `pub enum Value { Int(i64), Bool(bool) }` — alloc-free.
    - `pub enum RuntimeError { TypeMismatch(&'static str), DivideByZero }`.
    - `Value::add/sub/mul/div/eq` returning `Result<Value, RuntimeError>`.
      Wrap-on-overflow arithmetic matching the bytecode VM.
  - 7 unit tests in the lib's `mod tests` cover int round-trip,
    overflow wrap, sub/mul, div-by-zero, mixed Int+Bool type
    mismatch, bool eq, mixed-eq mismatch.
  - `#![allow(clippy::should_implement_trait)]` at module level
    documents that the `add`/`sub`/`mul` names ARE intentional
    (Result return precludes std::ops::Add).
  - README gains "Embedded runtime" section with build commands
    + a forward pointer to RES-100 for the actual cross-compile.
- 2026-04-17 verification:
  - `cd resilient-runtime && cargo build && cargo test
    && cargo clippy -- -D warnings` — all clean (7 tests pass).
  - `resilient/` crate untouched — verified across all four
    feature configs:
    - default: 217 unit + 1 golden + 12 smoke = 230 tests
    - `--features z3`: 225 + 1 + 13 = 239 tests
    - `--features lsp`: 221 + 1 + 12 + 3 lsp_smoke = 237 tests
    - `--features jit`: 219 + 1 + 12 = 232 tests
- 2026-04-17 follow-up tickets queued:
  - RES-100 — cross-compile against thumbv7em-none-eabihf.
  - RES-101 — `embedded-alloc` integration for Vec/String support.
  - RES-102 — cortex-m + cortex-m-rt example program.
  - RES-103 — CI job that runs the cross-compile.
