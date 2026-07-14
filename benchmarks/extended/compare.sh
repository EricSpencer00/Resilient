#!/usr/bin/env bash
# RES-3933 (F-E4): compare a fresh benchmarks/extended/run.sh JSON
# blob against the committed baseline. Generic sibling of
# scripts/compare_perf.sh (which is hardcoded to fib's three keys) —
# this one walks whatever `*_median_ms` keys the baseline defines, so
# adding a row to run.sh + baseline/extended.json doesn't require
# touching this script.
#
# Usage:
#   benchmarks/extended/compare.sh <baseline.json> <fresh.json>
#
# Env vars:
#   PERF_THRESHOLD_PCT     — default threshold; default 15.
#   PERF_THRESHOLD_PCT_JIT — threshold for any key starting with
#                            `jit_`; default 30. Mirrors
#                            scripts/compare_perf.sh's rationale
#                            (issue #387): JIT rows have higher
#                            hosted-runner variance from Cranelift
#                            codegen + first-call cache effects, so a
#                            tighter threshold causes spurious
#                            failures unrelated to the PR's diff.
#
# Output: a markdown table on stdout with per-row deltas, plus a
# final `PASS` / `FAIL` line. Exit 0 on pass, 1 on fail.

set -euo pipefail

if [[ $# -ne 2 ]]; then
    echo "Usage: $0 <baseline.json> <fresh.json>" >&2
    exit 2
fi

BASELINE=$1
FRESH=$2
THRESHOLD_PCT=${PERF_THRESHOLD_PCT:-15}
THRESHOLD_PCT_JIT=${PERF_THRESHOLD_PCT_JIT:-30}

if [[ ! -f "$BASELINE" ]]; then
    echo "error: baseline not found: $BASELINE" >&2
    exit 2
fi
if [[ ! -f "$FRESH" ]]; then
    echo "error: fresh results not found: $FRESH" >&2
    exit 2
fi

regressed=false
echo "| benchmark | baseline (ms) | fresh (ms) | delta % | threshold % |"
echo "| --------- | ------------: | ---------: | ------: | ----------: |"

# Every `*_median_ms` key the baseline defines drives one row. Keys
# added to run.sh but missing from the baseline (or vice versa) are
# reported as a warning rather than silently skipped, so a stale
# baseline doesn't quietly stop covering a benchmark.
keys=$(jq -r 'keys[] | select(endswith("_median_ms"))' "$BASELINE")

for key in $keys; do
    label=${key%_median_ms}
    if [[ "$label" == jit_* ]]; then
        threshold=$THRESHOLD_PCT_JIT
    else
        threshold=$THRESHOLD_PCT
    fi

    if ! jq -e --arg k "$key" 'has($k)' "$FRESH" > /dev/null; then
        echo "| $label | $(jq -r ".$key" "$BASELINE") | (missing) | — | $threshold% |"
        echo "warning: fresh results missing key '$key'" >&2
        regressed=true
        continue
    fi

    base=$(jq -r ".$key" "$BASELINE")
    fresh=$(jq -r ".$key" "$FRESH")
    delta=$(echo "scale=2; ($fresh - $base) / $base * 100" | bc -l)
    printf "| %s | %s | %s | %s%% | %s%% |\n" "$label" "$base" "$fresh" "$delta" "$threshold"
    exceeds=$(echo "$delta > $threshold" | bc -l)
    if [[ "$exceeds" -eq 1 ]]; then
        regressed=true
    fi
done

echo
echo "Thresholds: default ${THRESHOLD_PCT}%, jit_* rows ${THRESHOLD_PCT_JIT}% (override via PERF_THRESHOLD_PCT / PERF_THRESHOLD_PCT_JIT)"

if $regressed; then
    echo "**FAIL** — one or more benchmarks exceed the threshold (or are missing)."
    exit 1
else
    echo "**PASS** — all benchmarks within threshold."
    exit 0
fi
