#!/usr/bin/env bash
# Convenience wrapper. Prefer `cargo test` directly — golden tests
# under tests/examples_golden.rs run every example with a `.expected.txt`
# sidecar and fail on drift.
set -euo pipefail
cd "$(dirname "$0")"
exec cargo test
