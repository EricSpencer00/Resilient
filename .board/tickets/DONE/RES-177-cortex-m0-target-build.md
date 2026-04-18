---
id: RES-177
title: Cortex-M0 (thumbv6m-none-eabi) target: runtime builds clean
state: DONE
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
- 2026-04-17 claimed by executor
- 2026-04-17 done by executor

## Resolution
- `scripts/build_cortex_m0.sh` (new, executable): the
  ticket's three-command sequence plus an additional
  `--features alloc` build (the ticket Notes anticipated a
  fallback if alloc didn't link; it does, so we include it).
  Fail-fast shell; idempotent `rustup target add`.
- `.github/workflows/embedded.yml`: new `cortex_m0` job
  alongside the existing `cortex_m` and `riscv32` jobs.
  Same `dtolnay/rust-toolchain@master` + cache + script
  invocation pattern.
- `README.md`: new "Cortex-M0 / M0+ / M1 thumbv6m (RES-177)"
  subsection in the embedded section. Documents:
  - No atomics in the runtime today — RES-141's counters
    live in `resilient`, not `resilient-runtime`, so the
    ticket's `#[cfg(target_has_atomic = "32")]` gating
    isn't currently needed. CI watches for regressions.
  - `alloc` feature builds clean on M0.
  - `Value::Float(f64)` compiles (soft-float) but is slow
    — recommends staying on `Value::Int(i64)` when
    possible per the ticket Notes.
- Deviations: none. The ticket's "only include alloc if
  it links clean" clause resolved favourably — alloc works,
  so it's in.
- Verification:
  - `scripts/build_cortex_m0.sh` exits 0 locally (macOS
    host, stable rustc). All three sub-commands succeed.
  - Side-check: other two embedded scripts still pass
    unchanged (`build_cortex_m_demo.sh`, `build_riscv32.sh`).
  - Host-side regression: 468 `resilient` tests pass, 11 +
    14 `resilient-runtime` tests (no-alloc + alloc) pass.
