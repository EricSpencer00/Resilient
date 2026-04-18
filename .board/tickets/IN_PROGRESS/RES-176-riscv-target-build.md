---
id: RES-176
title: RISC-V rv32imac no_std target: `resilient-runtime` builds clean
state: IN_PROGRESS
priority: P3
goalpost: G16
created: 2026-04-17
owner: executor
---

## Summary
We proved Cortex-M4F in RES-097. RISC-V is the other major
embedded ISA. This ticket cross-compiles `resilient-runtime`
to `riscv32imac-unknown-none-elf` and confirms alloc-feature
builds too. No behavior change; a CI gate + documentation for
anyone targeting HiFive / GD32V / ESP32-C3 chips.

## Acceptance criteria
- Script `scripts/build_riscv32.sh` at repo root:
  ```sh
  rustup target add riscv32imac-unknown-none-elf
  cd resilient-runtime
  cargo build --release --target riscv32imac-unknown-none-elf
  cargo build --release --target riscv32imac-unknown-none-elf --features alloc
  cargo clippy --target riscv32imac-unknown-none-elf -- -D warnings
  ```
- `.github/workflows/embedded.yml` adds a job running the script
  alongside the existing Cortex-M build.
- README "Embedded" section lists RISC-V as a second supported
  target class.
- Commit message: `RES-176: rv32imac no_std build`.

## Notes
- If `embedded-alloc` isn't ready for rv32 at the version we're
  pinning, document the gap in the runtime crate's README and
  omit the alloc-feature build from the CI step for now — don't
  block the no-alloc build.
- No separate demo crate on RISC-V yet (RES-101 style) — one
  embedded demo is enough; multiple demos multiply maintenance.

## Log
- 2026-04-17 created by manager
- 2026-04-17 claimed by executor
