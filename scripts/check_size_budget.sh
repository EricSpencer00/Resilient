#!/usr/bin/env bash
# RES-179: code-size budget gate. Run `cargo bloat` on the
# Cortex-M4F demo release build, extract the `.text` section
# size, and fail if it exceeds the budget. The top-20-symbol
# table from bloat is printed either way so CI consumers can
# see the breakdown and, on failure, which crate / function
# drove the regression.
#
# Budget: 64 KiB by default (generous per the ticket; tighten
# in a follow-up once we have a stable baseline). Override via
# the `SIZE_BUDGET_KIB` environment variable.
#
# Usage:
#   scripts/check_size_budget.sh
#   SIZE_BUDGET_KIB=32 scripts/check_size_budget.sh
#
# Exits 0 when .text is under budget, 1 when over. Other
# failures (bloat / rustup) exit non-zero with the underlying
# error.

set -euo pipefail

BUDGET_KIB="${SIZE_BUDGET_KIB:-64}"

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT/resilient-runtime-cortex-m-demo"

# Make sure the target + cargo-bloat are present. rustup target
# add is idempotent; cargo install --locked with `--quiet`
# avoids spamming CI logs on repeat runs.
rustup target add thumbv7em-none-eabihf >/dev/null
command -v cargo-bloat >/dev/null || cargo install --locked cargo-bloat

# Run bloat, capturing output. `-n 20` caps the symbol table
# at 20 entries — enough detail to attribute regressions, not
# enough to bury the summary.
OUTPUT=$(cargo bloat --release --target thumbv7em-none-eabihf -n 20)

# Print the full report so the CI log shows it.
echo "$OUTPUT"
echo

# The summary row is the last line that begins with the
# crate / section total. Its shape is consistent:
#
#   1.5% 100.0% 2.3KiB     .text section size, the file size is 156.9KiB
#
# We want column 3 — the size-with-unit. `awk` handles the
# whitespace split cleanly.
SIZE_STR=$(echo "$OUTPUT" | awk '/\.text section size/ {print $3}')
if [[ -z "$SIZE_STR" ]]; then
    echo "check_size_budget: could not find `.text section size` row in bloat output" >&2
    exit 1
fi

# Parse a size like "2.3KiB" / "64.0KiB" / "1.2MiB" / "512B"
# into bytes. awk's floating-point arithmetic keeps sub-KiB
# precision; `int()` rounds down to a whole number of bytes.
NUM="${SIZE_STR//[!0-9.]/}"
UNIT="${SIZE_STR//[0-9.]/}"
case "$UNIT" in
    B)   BYTES=$(awk "BEGIN { print int($NUM) }") ;;
    KiB) BYTES=$(awk "BEGIN { print int($NUM * 1024) }") ;;
    MiB) BYTES=$(awk "BEGIN { print int($NUM * 1024 * 1024) }") ;;
    *)
        echo "check_size_budget: unknown size unit \"$UNIT\" in \"$SIZE_STR\"" >&2
        exit 1
        ;;
esac

BUDGET_BYTES=$((BUDGET_KIB * 1024))

echo "check_size_budget: .text = $SIZE_STR ($BYTES bytes); budget = $BUDGET_KIB KiB ($BUDGET_BYTES bytes)"
if (( BYTES > BUDGET_BYTES )); then
    echo >&2
    echo "check_size_budget: FAIL — .text exceeds the $BUDGET_KIB KiB budget." >&2
    echo "check_size_budget: see the symbol table above for attribution." >&2
    exit 1
fi
echo "check_size_budget: OK"
