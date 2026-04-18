---
id: RES-176
title: RISC-V rv32imac no_std target: `resilient-runtime` builds clean
state: DONE
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
- 2026-04-17 done by executor

## Resolution
- `scripts/build_riscv32.sh` (new, executable): installs the
  `riscv32imac-unknown-none-elf` target via `rustup target add`
  (idempotent no-op on repeat runs), then from
  `resilient-runtime/` runs:
  - `cargo build --release --target riscv32imac-unknown-none-elf`
  - `cargo build --release --target riscv32imac-unknown-none-elf --features alloc`
  - `cargo clippy --target riscv32imac-unknown-none-elf -- -D warnings`
  Exact command sequence the ticket specified. Shebang is
  `/usr/bin/env bash`, `set -euo pipefail` for fail-fast.
- `.github/workflows/embedded.yml` (new): two jobs, one per
  target class:
  - `cortex_m` runs the existing
    `scripts/build_cortex_m_demo.sh` (previously only
    invokable locally — now gated in CI alongside the new
    RISC-V path, which matches the ticket's "alongside the
    existing Cortex-M build" language).
  - `riscv32` runs the new `scripts/build_riscv32.sh`.
  - Both jobs use `dtolnay/rust-toolchain@master` with the
    matching `targets:` field so there's no rustup install
    step to maintain separately.
  - Standard cache step per job; concurrency group
    cancels in-flight runs on rebase.
- `README.md`: new "RISC-V rv32imac (RES-176)" subsection
  under "Embedded runtime" documenting the install +
  three-step build, the `scripts/build_riscv32.sh`
  shortcut, the CI gate, and the "no separate demo crate"
  decision from the ticket's Notes.
- `embedded-alloc` 0.5.1 (the version pinned in
  `resilient-runtime/Cargo.toml`) works for rv32imac
  out-of-the-box — no alloc-feature omission needed.
  `linked_list_allocator` + `critical-section` both pull
  in cleanly.
- Deviations: none.
- Verification:
  - `scripts/build_riscv32.sh` on macOS host — exits 0.
    Default target build, alloc-feature build, and
    clippy-deny-warnings all green.
  - `scripts/build_cortex_m_demo.sh` — still passes unchanged.
  - `resilient` host-side regression: 468 passed (no change).
  - `resilient-runtime` host-side: 11 no-alloc + 14 alloc-feature
    tests still pass (unchanged from RES-097 / RES-098).
  - `cargo clippy --target riscv32imac-unknown-none-elf --
    -D warnings` — clean.
