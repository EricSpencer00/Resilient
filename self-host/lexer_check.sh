#!/usr/bin/env bash
# RES-323: snapshot-based test driver for the self-hosted lexer.
#
# For every `self-host/lexer_tests/*.rz` input, runs `lexer.res`
# with `SELF_HOST_INPUT` pointing at the input, strips the
# interpreter's seed/footer noise, and diffs the result against the
# committed `*.expected.txt` snapshot.
#
# Exit codes:
#   0 — every test's token stream matches its snapshot
#   1 — one or more snapshots mismatched (unified diff printed)
#   2 — couldn't find the rz binary
#   3 — invocation problem (e.g. test directory missing)
#
# This is the lexer's local guardrail. The follow-up to RES-323 is
# wiring a dynamic cross-check against `lexer_logos.rs` output, at
# which point this snapshot harness becomes the inner sanity loop
# and the cross-check becomes the outer correctness gate.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"

TESTS_DIR="self-host/lexer_tests"
LEXER_SRC="self-host/lexer.res"

if [[ ! -d "$TESTS_DIR" ]]; then
    echo "error: test directory $TESTS_DIR not found" >&2
    exit 3
fi
if [[ ! -f "$LEXER_SRC" ]]; then
    echo "error: lexer source $LEXER_SRC not found" >&2
    exit 3
fi

BIN=""
if [[ -n "${RZ_BIN:-}" && -x "$RZ_BIN" ]]; then
    BIN="$RZ_BIN"
elif [[ -x resilient/target/debug/rz ]]; then
    BIN=resilient/target/debug/rz
elif [[ -x resilient/target/release/rz ]]; then
    BIN=resilient/target/release/rz
else
    echo "error: no rz binary found; build with 'cargo build --manifest-path resilient/Cargo.toml' or set RZ_BIN=/path/to/rz." >&2
    exit 2
fi

PASS=0
FAIL=0
FAILED_NAMES=()

for input in "$TESTS_DIR"/*.rz; do
    [[ -e "$input" ]] || continue
    name=$(basename "$input" .rz)
    expected="$TESTS_DIR/$name.expected.txt"
    if [[ ! -f "$expected" ]]; then
        echo "self-host: $name — MISSING expected file ($expected)" >&2
        FAIL=$((FAIL + 1))
        FAILED_NAMES+=("$name (no expected file)")
        continue
    fi

    actual=$(SELF_HOST_INPUT="$input" "$BIN" "$LEXER_SRC" 2>/dev/null \
                | grep -v "^seed=" \
                | grep -v "^Program executed successfully$")

    if diff -u "$expected" <(printf '%s\n' "$actual"); then
        echo "self-host: $name — OK"
        PASS=$((PASS + 1))
    else
        echo "self-host: $name — MISMATCH" >&2
        FAIL=$((FAIL + 1))
        FAILED_NAMES+=("$name")
    fi
done

echo
echo "self-host: $PASS passed, $FAIL failed"
if (( FAIL > 0 )); then
    echo "failed tests: ${FAILED_NAMES[*]}" >&2
    exit 1
fi
exit 0
