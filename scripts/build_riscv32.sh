#!/usr/bin/env bash
# RES-176: cross-compile `resilient-runtime` to the
# `riscv32imac-unknown-none-elf` target, proving the `no_std`
# runtime layer links clean on RISC-V (rv32imac — the baseline
# for HiFive, GD32V, ESP32-C3 class chips). We build BOTH the
# default (alloc-free) config and the `alloc` config, and run
# clippy with `-D warnings` so any RISC-V-specific lint lands
# as a CI failure rather than a silent regression.
#
# This is a build gate, not a runtime demonstration — we do NOT
# run the output under QEMU (separate runner setup). The demo
# crate from RES-101 stays Cortex-M-only per the ticket Notes;
# one embedded demo is enough.
#
# Usage:
#   scripts/build_riscv32.sh
#
# Exits 0 on success, non-zero on any build / clippy failure.

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT/resilient-runtime"

# `rustup target add` is idempotent — silent no-op on repeat runs.
rustup target add riscv32imac-unknown-none-elf >/dev/null

# Default: alloc-free. Proves Value::Int / Value::Bool types
# and core ops work without any allocator on-target.
cargo build --release --target riscv32imac-unknown-none-elf

# With --features alloc: pulls in embedded-alloc (via
# linked_list_allocator). Proves Value::String / heap-backed
# Value::Float variants build for RISC-V.
cargo build --release --target riscv32imac-unknown-none-elf --features alloc

# Clippy with deny-warnings so a rv32-specific lint doesn't slip
# through — runs against the default feature set (clippy does
# not re-check --features unless asked).
cargo clippy --target riscv32imac-unknown-none-elf -- -D warnings
