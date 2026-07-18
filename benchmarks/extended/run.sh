#!/usr/bin/env bash
# RES-3933 (F-E4): extended perf-gate benchmark runner.
#
# fib(25) (benchmarks/fib/run.sh) is the only workload the perf gate
# measured before this ticket. This script adds a second, still-cheap
# slice across the OTHER benchmark dirs that already exist in the
# repo, so the gate catches regressions outside the fib/dispatch hot
# path too:
#
#   - benchmarks/sum            — loop/arithmetic overhead, walker only.
#   - benchmarks/vm             — RES-172 peephole workload, --vm only.
#                                  counter_loop.rs's 1,000,000-iteration
#                                  while loop trips the tree walker's
#                                  runaway-loop guard ("while loop
#                                  exceeded 1000000 iterations") — it
#                                  errors out in walker mode by design
#                                  of that guard, not a bug in this
#                                  script, so there is no walker row
#                                  for this benchmark.
#   - benchmarks/jit            — RES-175 inliner + RES-168 TCO
#                                  microbenches. JIT-only: tail_rec.rs
#                                  crashes the tree walker by design
#                                  (see benchmarks/jit/RESULTS.md), so
#                                  it is never run outside --jit.
#   - benchmarks/contracts       — runtime `requires` overhead, walker
#                                  only.
#
# Deliberately NOT included: benchmarks/lex and benchmarks/compile_time.
# Both READMEs already say so in as many words ("Contributor infra;
# not a CI gate") because a single lex-bench pass is ~18s and a cold
# compile_time run is ~5 minutes — multiple orders of magnitude over
# every other row here. Keeping them out of CI is a deliberate
# time-budget decision, not an oversight.
#
# Reuses whatever release binaries are already on disk (built by
# benchmarks/fib/run.sh in the same perf-gate job) and only builds
# them itself when run standalone.
#
# Usage:
#   ./benchmarks/extended/run.sh
#
# Output: a single JSON object on stdout, e.g.
#   { "sum_interp_median_ms": 12.3,
#     "vm_counter_loop_median_ms": 7.8,
#     "jit_leaf_heavy_median_ms": 9.1,
#     "jit_tail_rec_median_ms": 4.4,
#     "contracts_with_median_ms": 148.7,
#     "contracts_without_median_ms": 124.9,
#     "system": "Darwin arm64", "date": "2026-07-13T00:00:00Z" }
#
# 10 samples / 3 warmups per row (vs. fib's 20/5) — six rows here
# vs. fib's three, so the sample count is trimmed to keep total
# perf-gate wall time bounded. Exit 0 on success, non-zero if any
# backend failed to run.
#
# RES-4108: the two JIT rows (jit_leaf_heavy, jit_tail_rec) are
# split into their OWN hyperfine invocation at fib's tuned 5
# warmups / 20 runs instead of this file's shared 3/10. Both are
# sub-10ms micro-benchmarks where per-run cost is dominated by a
# roughly-fixed JIT-compile / first-call cost rather than the
# workload itself, so a single hosted-runner hiccup (GC pause, CPU
# contention) during one of only 10 samples can drag the median
# 40x (see issue #4108: jit_tail_rec's baseline 2.618ms measured as
# 110.284ms on one run, 2.822ms on an immediate re-run of the same
# commit). This is exactly the failure mode issue #387 already
# fixed for the fib(25) JIT row — bumping warmups absorbs the
# cold-cache/contention event and bumping samples makes the median
# itself robust to one bad sample — so the same 5/20 tuning is
# applied here rather than inventing a new statistic. The other
# four rows (sum/vm/contracts, all >100ms workloads where fixed
# overhead is proportionally tiny) keep the cheaper 3/10 to bound
# total perf-gate wall time.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$REPO_ROOT"

RES_DIR=resilient
RES=$RES_DIR/target/release/rz
RES_JIT=$RES_DIR/target/release/rz-with-jit

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
    --export-json "$TMP/extended.json" \
    --warmup 3 \
    --runs 10 \
    --style none \
    --command-name "sum_interp"              "$RES --seed $SEED benchmarks/sum/sum.rs" \
    --command-name "vm_counter_loop"          "$RES --seed $SEED --vm benchmarks/vm/counter_loop.rs" \
    --command-name "contracts_with"           "$RES --seed $SEED benchmarks/contracts/with_contract.rs" \
    --command-name "contracts_without"        "$RES --seed $SEED benchmarks/contracts/no_contract.rs" \
    > /dev/null

# RES-4108: JIT rows get fib's tuned 5 warmups / 20 runs — see the
# header comment above for why these two rows specifically need it.
hyperfine \
    --export-json "$TMP/extended-jit.json" \
    --warmup 5 \
    --runs 20 \
    --style none \
    --command-name "jit_leaf_heavy"           "$RES_JIT --seed $SEED --jit benchmarks/jit/leaf_heavy.rs" \
    --command-name "jit_tail_rec"             "$RES_JIT --seed $SEED --jit benchmarks/jit/tail_rec.rs" \
    > /dev/null

median_ms() {
    jq --arg name "$1" '.results[] | select(.command==$name) | (.median * 1000 * 1000 | round) / 1000' "$TMP/extended.json"
}

median_ms_jit() {
    jq --arg name "$1" '.results[] | select(.command==$name) | (.median * 1000 * 1000 | round) / 1000' "$TMP/extended-jit.json"
}

sum_interp=$(median_ms "sum_interp")
vm_counter_loop=$(median_ms "vm_counter_loop")
jit_leaf=$(median_ms_jit "jit_leaf_heavy")
jit_tail=$(median_ms_jit "jit_tail_rec")
contracts_with=$(median_ms "contracts_with")
contracts_without=$(median_ms "contracts_without")

system="$(uname -sm)"
date_utc="$(date -u +%Y-%m-%dT%H:%M:%SZ)"

jq -n \
    --argjson sum_interp "$sum_interp" \
    --argjson vm_counter_loop "$vm_counter_loop" \
    --argjson jit_leaf "$jit_leaf" \
    --argjson jit_tail "$jit_tail" \
    --argjson contracts_with "$contracts_with" \
    --argjson contracts_without "$contracts_without" \
    --arg system "$system" \
    --arg date "$date_utc" \
    '{
        sum_interp_median_ms: $sum_interp,
        vm_counter_loop_median_ms: $vm_counter_loop,
        jit_leaf_heavy_median_ms: $jit_leaf,
        jit_tail_rec_median_ms: $jit_tail,
        contracts_with_median_ms: $contracts_with,
        contracts_without_median_ms: $contracts_without,
        system: $system,
        date: $date
    }'
