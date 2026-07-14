#!/usr/bin/env bash
# RES-3987 (D-E1): build the on-device `.rzbc` loader binary,
# proving `resilient-runtime`'s `vm::loader::load_and_run` links and
# cross-compiles clean under `#![no_std]`. Mirrors
# `build_cortex_m_demo.sh`'s shape. We do NOT run the output under
# QEMU here — that's the follow-up `embedded-runtime.yml` CI job
# `docs/EMBEDDED_PIPELINE.md` section 4 describes.
#
# Usage:
#   scripts/build_loader_demo.sh
#
# Exits 0 on success, non-zero on any build / clippy failure.

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT/resilient-runtime-loader-demo"

rustup target add thumbv7em-none-eabihf >/dev/null

cargo build --release --target thumbv7em-none-eabihf
# No `--all-targets` here: clippy's implicit `--tests` pass would
# try to link a `test`-harness binary against this crate's
# `#![no_std] #![no_main]` root, which fails with "can't find crate
# for `test`" — the same reason `build_cortex_m_demo.sh` omits it.
cargo clippy --release --target thumbv7em-none-eabihf -- -D warnings
