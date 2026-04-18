#!/usr/bin/env bash
# RES-202: compare fresh fib benchmark JSON against the baseline.
#
# Usage:
#   scripts/compare_perf.sh <baseline.json> <fresh.json>
#
# Env vars:
#   PERF_THRESHOLD_PCT — threshold percent; default 15. Fails if
#                       any median is >THRESHOLD slower than
#                       baseline.
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

if [[ ! -f "$BASELINE" ]]; then
    echo "error: baseline not found: $BASELINE" >&2
    exit 2
fi
if [[ ! -f "$FRESH" ]]; then
    echo "error: fresh results not found: $FRESH" >&2
    exit 2
fi

regressed=false
echo "| backend | baseline (ms) | fresh (ms) | delta % |"
echo "| ------- | ------------: | ---------: | ------: |"

for key in walker_median_ms vm_median_ms jit_median_ms; do
    label=${key%_median_ms}
    base=$(jq -r ".$key" "$BASELINE")
    fresh=$(jq -r ".$key" "$FRESH")
    # Percentage delta. bc -l for floating-point.
    delta=$(echo "scale=2; ($fresh - $base) / $base * 100" | bc -l)
    printf "| %s | %s | %s | %s%% |\n" "$label" "$base" "$fresh" "$delta"
    # $delta > $THRESHOLD → regression.
    exceeds=$(echo "$delta > $THRESHOLD_PCT" | bc -l)
    if [[ "$exceeds" -eq 1 ]]; then
        regressed=true
    fi
done

echo
echo "Threshold: ${THRESHOLD_PCT}% (set PERF_THRESHOLD_PCT to override)"

if $regressed; then
    echo "**FAIL** — one or more backends exceed the threshold."
    exit 1
else
    echo "**PASS** — all backends within threshold."
    exit 0
fi
