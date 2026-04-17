---
id: RES-177
title: Cortex-M0 (thumbv6m-none-eabi) target: runtime builds clean
state: OPEN
priority: P3
goalpost: G16
created: 2026-04-17
owner: executor
---

## Summary
M0 lacks M4F's 32-bit atomics and FPU. Proving the runtime builds
for thumbv6m catches any future dep creep that assumes those. Same
approach as RES-097 / RES-176: script + CI gate.

## Acceptance criteria
- Script `scripts/build_cortex_m0.sh`:
  ```sh
  rustup target add thumbv6m-none-eabi
  cd resilient-runtime
  cargo build --release --target thumbv6m-none-eabi
  cargo clippy --target thumbv6m-none-eabi -- -D warnings
  ```
- If the runtime uses 32-bit atomics anywhere (check
  RES-141's counters), gate that code behind `#[cfg(target_has_atomic
  = "32")]` and offer an alternative — document clearly in the
  crate README.
- CI job in `.github/workflows/embedded.yml`.
- Commit message: `RES-177: thumbv6m-none-eabi runtime build`.

## Notes
- alloc on M0: possible with `embedded-alloc`; if it links clean
  we include the alloc build too, otherwise no-alloc only. Either
  way, document the outcome.
- `Value::Float(f64)` compiles on M0 (soft-float via libgcc);
  confirmed fine but flag in README that floats are slow on M0.

## Log
- 2026-04-17 created by manager
