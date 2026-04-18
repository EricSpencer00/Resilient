#!/usr/bin/env bash
# RES-202: fib(25) micro-bench runner. Produces JSON on stdout
# with the median wall-clock for each of the three Resilient
# backends (walker, VM, JIT). Consumed by the perf-gate CI
# workflow and by `scripts/update_perf_baseline.sh`.
#
# Runs hyperfine on 10 samples × 3 warmup-runs for each backend.
# Wall-clock is the usual "my program, start to finish" time;
# compile time is amortized into it for the VM/JIT rows, which
# is appropriate for the `fib(25)` workload (small setup cost,
# dominant bench loop).
#
# Output shape (single JSON object on stdout):
#   { "walker_median_ms": 42.1,
#     "vm_median_ms": 18.6,
#     "jit_median_ms": 9.3,
#     "system": "Darwin arm64", "date": "2026-04-17T12:34:56Z" }
#
# Exit 0 on success, non-zero if any backend failed to run.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$REPO_ROOT"

# Build artefacts. The default-features binary is used for the
# walker + VM rows (they don't need the JIT deps); a separate
# `--features jit` build drives the JIT row.
RES_DIR=resilient
RES=$RES_DIR/target/release/resilient
RES_JIT=$RES_DIR/target/release/resilient-with-jit

if [[ ! -x "$RES" ]]; then
    (cd "$RES_DIR" && cargo build --release --locked --quiet)
fi
if [[ ! -x "$RES_JIT" ]]; then
    (cd "$RES_DIR" \
        && cargo build --release --features jit --locked --quiet \
        && cp target/release/resilient target/release/resilient-with-jit)
fi

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

# Seed is fixed so the runtime's RNG init doesn't add jitter
# between runs (relevant for the walker row, which boots the
# RNG before executing; VM and JIT share the same startup).
SEED=0

hyperfine \
    --export-json "$TMP/fib.json" \
    --warmup 3 \
    --runs 10 \
    --style none \
    --command-name "walker" "$RES --seed $SEED benchmarks/fib/fib.rs" \
    --command-name "vm"     "$RES --seed $SEED --vm benchmarks/fib/fib_vm.rs" \
    --command-name "jit"    "$RES_JIT --seed $SEED --jit benchmarks/fib/fib_jit.rs" \
    > /dev/null

# Hyperfine reports median in seconds; convert to milliseconds
# with a 3-decimal rounding to keep the JSON human-readable.
walker_ms=$(jq '.results[] | select(.command=="walker") | (.median * 1000 * 1000 | round) / 1000' "$TMP/fib.json")
vm_ms=$(jq     '.results[] | select(.command=="vm")     | (.median * 1000 * 1000 | round) / 1000' "$TMP/fib.json")
jit_ms=$(jq    '.results[] | select(.command=="jit")    | (.median * 1000 * 1000 | round) / 1000' "$TMP/fib.json")

system="$(uname -sm)"
date_utc="$(date -u +%Y-%m-%dT%H:%M:%SZ)"

jq -n \
    --argjson walker "$walker_ms" \
    --argjson vm "$vm_ms" \
    --argjson jit "$jit_ms" \
    --arg system "$system" \
    --arg date "$date_utc" \
    '{walker_median_ms: $walker, vm_median_ms: $vm, jit_median_ms: $jit, system: $system, date: $date}'
