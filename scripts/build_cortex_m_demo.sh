#!/usr/bin/env bash
# RES-101: build the Cortex-M4F demo crate, proving
# `resilient-runtime` links on-target with `embedded-alloc`'s
# `LlffHeap` as the global allocator. Running clean is the
# onboarding evidence this ticket exists to produce — we do NOT
# run the output under QEMU here (that would need a separate
# runner setup).
#
# Usage:
#   scripts/build_cortex_m_demo.sh
#
# Exits 0 on success, non-zero on any build / clippy failure.

set -euo pipefail

# Resolve the repo root regardless of where the script is invoked
# from (Makefiles, CI steps, IDEs, etc.).
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT/resilient-runtime-cortex-m-demo"

# `rustup target add` is idempotent — no-op when the toolchain
# already has the target. Silence the install message so the
# common case (repeat CI runs) doesn't spam logs.
rustup target add thumbv7em-none-eabihf >/dev/null

cargo build --release --target thumbv7em-none-eabihf
cargo clippy --release --target thumbv7em-none-eabihf -- -D warnings
