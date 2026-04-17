#!/bin/bash
# Deprecated: prefer `cargo test` — golden tests under tests/examples_golden.rs
# run every example with a `.expected.txt` sidecar and fail on drift.
#
# This wrapper is kept so existing docs/muscle memory still work.

set -euo pipefail
cd "$(dirname "$0")"

echo "Note: this script now delegates to \`cargo test\`."
echo "Use \`cargo test -- --ignored missing_expected_files\` to see which"
echo "examples still need golden-output files."
echo

exec cargo test
