#!/usr/bin/env bash
# RES-4447: measure JIT startup for a program with many distinct functions.
# This isolates the metadata-lowering cost that is hidden by runtime-heavy
# benchmarks. The fixture's result is checked before timing so a faster run
# cannot mask a correctness regression.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$REPO_ROOT"

RES_DIR=resilient
RES_JIT="${RES_JIT:-$RES_DIR/target/release/rz-with-jit}"
PROGRAM=benchmarks/jit_startup/function_table.rz
EXPECTED=512
SEED=0

if [[ ! -x "$RES_JIT" ]]; then
    (cd "$RES_DIR" \
        && cargo build --release --features jit --locked --quiet \
        && cp target/release/rz target/release/rz-with-jit)
fi

output="$("$RES_JIT" --seed "$SEED" --jit "$PROGRAM" 2>/dev/null)"
actual="${output%%$'\n'*}"
if [[ "$actual" != "$EXPECTED" ]]; then
    echo "expected $EXPECTED from $PROGRAM, got $actual" >&2
    exit 1
fi

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

if command -v hyperfine >/dev/null 2>&1; then
    hyperfine \
        --shell=none \
        --export-json "$TMP/function_table.json" \
        --warmup 5 \
        --runs 20 \
        --style none \
        --command-name "jit_512_functions" \
        "$RES_JIT --seed $SEED --jit $PROGRAM >/dev/null 2>/dev/null" \
        > /dev/null
    median_ms=$(jq '.results[] | select(.command=="jit_512_functions") | (.median * 1000 * 1000 | round) / 1000' "$TMP/function_table.json")
    samples=20
else
    # Keep the benchmark runnable on minimal build hosts that do not package
    # hyperfine; CI and developer machines still use the lower-noise path.
    for _ in 1 2 3 4 5; do
        /usr/bin/time -f "%e" \
            sh -c 'exec "$1" --seed "$2" --jit "$3" >/dev/null 2>/dev/null' \
            sh "$RES_JIT" "$SEED" "$PROGRAM" 2>>"$TMP/times"
    done
    median_ms=$(sort -n "$TMP/times" | awk 'NR==3 { printf "%.3f", $1 * 1000 }')
    samples=5
fi

system="$(uname -sm)"
date_utc="$(date -u +%Y-%m-%dT%H:%M:%SZ)"

jq -n \
    --argjson median_ms "$median_ms" \
    --argjson function_count "$EXPECTED" \
    --argjson samples "$samples" \
    --arg system "$system" \
    --arg date "$date_utc" \
    '{
        function_table_jit_median_ms: $median_ms,
        function_count: $function_count,
        samples: $samples,
        expected_result: 512,
        system: $system,
        date: $date
    }'
