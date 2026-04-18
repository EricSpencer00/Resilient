#!/usr/bin/env bash
# RES-177: cross-compile `resilient-runtime` to
# `thumbv6m-none-eabi` — Cortex-M0 / M0+ / M1, the lowest-common-
# denominator Armv6-M ISA. Proves the runtime doesn't lean on
# M4F-only features (32-bit atomics, FPU, DSP). M0 will stay on
# this watch so a future dep creep (e.g. "pulled in a crate
# assuming `AtomicU32`") fails in CI rather than in the field.
#
# The runtime today uses NO atomics at all (RES-141's counters
# live in the `resilient` CLI binary, not the runtime crate), so
# there's no target_has_atomic gating to do. If that ever
# changes, the ticket's checklist requires `#[cfg(target_has_atomic
# = "32")]` gating + a README note.
#
# `embedded-alloc` 0.5.1 + `linked_list_allocator` + `critical-
# section` all link clean on thumbv6m (critical-section's M0
# single-core backend is stock), so we include the alloc-feature
# build too — matches the RES-176 RISC-V pattern, unlike the
# fallback path the ticket Notes reserved for the case where
# alloc didn't link.
#
# Usage:
#   scripts/build_cortex_m0.sh
#
# Exits 0 on success, non-zero on any build / clippy failure.

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT/resilient-runtime"

rustup target add thumbv6m-none-eabi >/dev/null

cargo build --release --target thumbv6m-none-eabi
cargo build --release --target thumbv6m-none-eabi --features alloc
cargo clippy --target thumbv6m-none-eabi -- -D warnings
