#!/usr/bin/env bash
# RES-202: compare fresh fib benchmark JSON against the baseline.
#
# Usage:
#   scripts/compare_perf.sh <baseline.json> <fresh.json>
#
# Env vars:
#   PERF_THRESHOLD_PCT     — default threshold (walker, VM); default 15.
#   PERF_THRESHOLD_PCT_JIT — JIT threshold; default 30. The JIT has
#                            higher hosted-runner variance from
#                            LLVM codegen + first-call cache; per
#                            issue #387 a tighter threshold causes
#                            spurious failures on documentation-only
#                            PRs. The slack is bookkeeping for the
#                            measurement noise floor, not permission
#                            to regress.
#
# Output: a markdown table on stdout with per-backend deltas,
# plus a final `PASS` / `FAIL` line. Exit 0 on pass, 1 on fail.
# The table is suitable for a PR comment; perf_gate.yml pipes
# it into `actions/github-script`.

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
echo "| backend | baseline (ms) | fresh (ms) | delta % | threshold % |"
echo "| ------- | ------------: | ---------: | ------: | ----------: |"

for key in walker_median_ms vm_median_ms jit_median_ms; do
    label=${key%_median_ms}
    if [[ "$label" == "jit" ]]; then
        threshold=$THRESHOLD_PCT_JIT
    else
        threshold=$THRESHOLD_PCT
    fi
    base=$(jq -r ".$key" "$BASELINE")
    fresh=$(jq -r ".$key" "$FRESH")
    # Percentage delta. bc -l for floating-point.
    delta=$(echo "scale=2; ($fresh - $base) / $base * 100" | bc -l)
    printf "| %s | %s | %s | %s%% | %s%% |\n" "$label" "$base" "$fresh" "$delta" "$threshold"
    # $delta > $threshold → regression.
    exceeds=$(echo "$delta > $threshold" | bc -l)
    if [[ "$exceeds" -eq 1 ]]; then
        regressed=true
    fi
done

echo
echo "Thresholds: walker/vm ${THRESHOLD_PCT}%, jit ${THRESHOLD_PCT_JIT}% (override via PERF_THRESHOLD_PCT / PERF_THRESHOLD_PCT_JIT)"

if $regressed; then
    echo "**FAIL** — one or more backends exceed the threshold."
    exit 1
else
    echo "**PASS** — all backends within threshold."
    exit 0
fi
