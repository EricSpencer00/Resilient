#!/usr/bin/env bash
# RES-4134: native-vs-fallback coverage sweep for the JIT backend.
#
# The JIT differential pass (resilient/tests/it/differential.rs,
# RES-4111) proves interpreter/--jit *output* parity across the
# example corpus, but says nothing about *how* --jit got there: RES-
# 4019's transparent fallback means a passing row may never have
# touched Cranelift at all. This script answers the "how much of the
# corpus actually executes through native lowering today" question
# that #4134 asks for, so future string/struct lowering PRs have a
# baseline number to move.
#
# Caveat discovered while building this script: the dominant cause
# of fallback across the corpus today is not string literals per se
# but `println(...)` itself — the JIT has no builtin-call lowering
# at all yet ("jit: unsupported: call to unknown function"), so a
# purely-arithmetic example that merely *prints* its result still
# falls back. String-literal/op lowering (#4134 item 1) won't move
# this number much on its own; builtin-call lowering is the bigger
# remaining gap. Kept as a script comment rather than a filed
# ticket here since #4134 already tracks the general "native
# lowering remainder" scope.
#
# Detection method: `--jit --verbose` emits exactly one line to
# stderr — "note: --jit fell back to the VM for FILE: REASON" — from
# the `JitError::is_precompile()` branch in `resilient/src/lib.rs`
# (RES-4019), and nothing else identifying the backend choice on
# success. Its absence on a zero-exit run means the program executed
# entirely through native Cranelift lowering. `--verbose` also turns
# on the typechecker (implies `--typecheck`), so a handful of examples
# that are deliberately not type-clean will error out here; those are
# counted separately as "errored" rather than folded into native or
# fallback, since neither backend actually ran the workload.
#
# Usage:
#   ./benchmarks/jit_startup/coverage.sh
#
# Output: a single JSON object on stdout, e.g.
#   { "native": 812, "fallback": 5, "errored": 12, "total": 829,
#     "native_pct": 97.9, "system": "Darwin arm64",
#     "date": "2026-07-18T00:00:00Z" }

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$REPO_ROOT"

RES_DIR=resilient
RES_JIT=$RES_DIR/target/release/rz-with-jit

if [[ ! -x "$RES_JIT" ]]; then
    (cd "$RES_DIR" \
        && cargo build --release --features jit --locked --quiet \
        && cp target/release/rz target/release/rz-with-jit)
fi

native=0
fallback=0
errored=0
total=0

for f in "$RES_DIR"/examples/*.rz; do
    total=$((total + 1))
    stderr=$(mktemp)
    if ! "$RES_JIT" --seed 0 --jit --verbose "$f" >/dev/null 2>"$stderr"; then
        errored=$((errored + 1))
        rm -f "$stderr"
        continue
    fi
    if grep -q "fell back to the VM" "$stderr"; then
        fallback=$((fallback + 1))
    else
        native=$((native + 1))
    fi
    rm -f "$stderr"
done

native_pct=$(echo "scale=1; 100 * $native / $total" | bc)
system="$(uname -sm)"
date_utc="$(date -u +%Y-%m-%dT%H:%M:%SZ)"

jq -n \
    --argjson native "$native" \
    --argjson fallback "$fallback" \
    --argjson errored "$errored" \
    --argjson total "$total" \
    --argjson native_pct "$native_pct" \
    --arg system "$system" \
    --arg date "$date_utc" \
    '{native: $native, fallback: $fallback, errored: $errored, total: $total, native_pct: $native_pct, system: $system, date: $date}'
