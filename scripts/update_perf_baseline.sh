#!/usr/bin/env bash
# RES-202: regenerate the perf-gate baseline from a fresh local
# run. Human-gated: the resulting commit is reviewed before
# landing, so this script only writes the file and prints a
# helpful reminder.
#
# Usage:
#   scripts/update_perf_baseline.sh
#
# After running: inspect `benchmarks/baseline/fib.json`, verify
# the deltas vs the prior baseline are expected, then
# `git add + commit`. The baseline bake-off is a HUMAN decision
# — don't commit silently.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"

mkdir -p benchmarks/baseline
OUT=benchmarks/baseline/fib.json
PRIOR=""
if [[ -f "$OUT" ]]; then
    PRIOR=$(cat "$OUT")
fi

echo "Running fib benchmark — this takes ~30 seconds..." >&2
./benchmarks/fib/run.sh > "$OUT"

echo >&2
echo "New baseline written to $OUT:" >&2
cat "$OUT" >&2
echo >&2

if [[ -n "$PRIOR" ]]; then
    echo "Prior baseline:" >&2
    echo "$PRIOR" >&2
    echo >&2
fi

echo "Review the deltas, then run:" >&2
echo "  git add $OUT" >&2
echo "  git commit -m 'perf: update fib baseline'" >&2
echo >&2
echo "Do NOT push without a human review of the new numbers." >&2
