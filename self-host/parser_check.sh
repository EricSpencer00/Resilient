#!/usr/bin/env bash
# RES-379: Self-hosting parser snapshot harness.
#
# For each *.tokens.txt in self-host/parser_tests/ that has a sibling
# *.expected.txt, lex the tokens through parser.rz and diff against the
# committed snapshot. Exit non-zero on any mismatch.
#
# Usage:
#   bash self-host/parser_check.sh
#   RZ_BIN=/path/to/rz bash self-host/parser_check.sh

set -euo pipefail

RZ_BIN="${RZ_BIN:-$(cargo build --manifest-path resilient/Cargo.toml -q 2>/dev/null; echo resilient/target/debug/rz)}"
TESTS_DIR="self-host/parser_tests"
PARSER="self-host/parser.rz"

pass=0
fail=0
skip=0

for tokens_file in "$TESTS_DIR"/*.tokens.txt; do
    stem="${tokens_file%.tokens.txt}"
    expected_file="${stem}.expected.txt"

    if [ ! -f "$expected_file" ]; then
        echo "  SKIP  $(basename "$tokens_file") — no expected file"
        skip=$((skip + 1))
        continue
    fi

    actual=$(SELF_HOST_TOKENS="$tokens_file" "$RZ_BIN" "$PARSER" 2>/dev/null \
        | grep -v '^seed=' \
        | grep -v '^Program executed successfully$')

    expected=$(cat "$expected_file")

    if [ "$actual" = "$expected" ]; then
        echo "  PASS  $(basename "$stem")"
        pass=$((pass + 1))
    else
        echo "  FAIL  $(basename "$stem")"
        echo "    expected: $expected"
        echo "    actual:   $actual"
        fail=$((fail + 1))
    fi
done

echo ""
echo "Results: $pass passed, $fail failed, $skip skipped"
[ "$fail" -eq 0 ]
