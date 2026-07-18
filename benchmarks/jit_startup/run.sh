#!/usr/bin/env bash
# RES-4134 (B-E4 follow-up): JIT startup-latency + peak-memory
# benchmark. `benchmarks/fib` and `benchmarks/jit` measure *workload*
# time (fib(25), tail recursion, leaf inlining) where the fixed cost
# of standing up a Cranelift module is a small fraction of the total.
# This benchmark isolates that fixed cost: `trivial.rz` is a single
# top-level `return 0;` with no calls, loops, or literals worth
# executing, so end-to-end wall time is (almost) entirely JIT
# module/ISA setup + single-block codegen + finalization, not
# workload execution. Contrast rows for the walker and VM are
# included for scale, not because either has meaningful "startup"
# overhead of its own.
#
# Peak RSS is measured with `/usr/bin/time -l` (BSD/macOS) or
# `/usr/bin/time -v` (GNU/Linux) since hyperfine doesn't report
# memory. Parsed out of a single warm run per backend (not
# averaged — RSS is far less noisy than wall-clock across runs).
#
# Usage:
#   ./benchmarks/jit_startup/run.sh
#
# Output: a single JSON object on stdout, e.g.
#   { "walker_median_ms": 8.1, "vm_median_ms": 6.9, "jit_median_ms": 4.3,
#     "walker_peak_rss_kb": 3120, "vm_peak_rss_kb": 3204,
#     "jit_peak_rss_kb": 9876,
#     "system": "Darwin arm64", "date": "2026-07-18T00:00:00Z" }
#
# Exit 0 on success, non-zero if any backend failed to run.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$REPO_ROOT"

RES_DIR=resilient
RES=$RES_DIR/target/release/rz
RES_JIT=$RES_DIR/target/release/rz-with-jit
PROGRAM=benchmarks/jit_startup/trivial.rz

if [[ ! -x "$RES" ]]; then
    (cd "$RES_DIR" && cargo build --release --locked --quiet)
fi
if [[ ! -x "$RES_JIT" ]]; then
    (cd "$RES_DIR" \
        && cargo build --release --features jit --locked --quiet \
        && cp target/release/rz target/release/rz-with-jit)
fi

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

SEED=0

hyperfine \
    --shell=none \
    --export-json "$TMP/jit_startup.json" \
    --warmup 5 \
    --runs 20 \
    --style none \
    --command-name "walker" "$RES --seed $SEED $PROGRAM" \
    --command-name "vm"     "$RES --seed $SEED --vm $PROGRAM" \
    --command-name "jit"    "$RES_JIT --seed $SEED --jit $PROGRAM" \
    > /dev/null

walker_ms=$(jq '.results[] | select(.command=="walker") | (.median * 1000 * 1000 | round) / 1000' "$TMP/jit_startup.json")
vm_ms=$(jq     '.results[] | select(.command=="vm")     | (.median * 1000 * 1000 | round) / 1000' "$TMP/jit_startup.json")
jit_ms=$(jq    '.results[] | select(.command=="jit")    | (.median * 1000 * 1000 | round) / 1000' "$TMP/jit_startup.json")

# Peak RSS, one warm run per backend. `/usr/bin/time -l` (macOS/BSD)
# reports "maximum resident set size" in bytes; GNU `/usr/bin/time -v`
# reports "Maximum resident set size (kbytes)" in KB directly.
peak_rss_kb() {
    local cmd=("$@")
    local out
    out=$(/usr/bin/time -l "${cmd[@]}" 2>&1 1>/dev/null) || true
    if echo "$out" | grep -q "maximum resident set size"; then
        # bytes -> KB
        echo "$out" | grep "maximum resident set size" | awk '{print int($1/1024)}'
        return
    fi
    out=$(/usr/bin/time -v "${cmd[@]}" 2>&1 1>/dev/null) || true
    if echo "$out" | grep -qi "Maximum resident set size"; then
        echo "$out" | grep -i "Maximum resident set size" | awk '{print $NF}'
        return
    fi
    echo "0"
}

walker_rss=$(peak_rss_kb "$RES" --seed "$SEED" "$PROGRAM")
vm_rss=$(peak_rss_kb "$RES" --seed "$SEED" --vm "$PROGRAM")
jit_rss=$(peak_rss_kb "$RES_JIT" --seed "$SEED" --jit "$PROGRAM")

system="$(uname -sm)"
date_utc="$(date -u +%Y-%m-%dT%H:%M:%SZ)"

jq -n \
    --argjson walker "$walker_ms" \
    --argjson vm "$vm_ms" \
    --argjson jit "$jit_ms" \
    --argjson walker_rss "$walker_rss" \
    --argjson vm_rss "$vm_rss" \
    --argjson jit_rss "$jit_rss" \
    --arg system "$system" \
    --arg date "$date_utc" \
    '{
        walker_median_ms: $walker,
        vm_median_ms: $vm,
        jit_median_ms: $jit,
        walker_peak_rss_kb: $walker_rss,
        vm_peak_rss_kb: $vm_rss,
        jit_peak_rss_kb: $jit_rss,
        system: $system,
        date: $date
    }'
