#!/usr/bin/env bash
# RES-4245 / RES-4855 regression test: perf comparisons must accept the
# fastest of multiple independent candidates while still rejecting a
# reproduced regression and missing measurements. The counter-loop cases
# also pin the reviewed Linux rebaseline against its existing 30% CI budget.

set -euo pipefail

REPO_ROOT="$(git rev-parse --show-toplevel)"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

fail() {
  echo "FAIL: $1" >&2
  exit 1
}

expect_pass() {
  local description="$1"
  shift
  local output
  output="$($@)" || fail "$description unexpectedly failed"
  grep -q '\*\*PASS\*\*' <<<"$output" || fail "$description did not report PASS"
  echo "PASS: $description"
}

expect_fail() {
  local description="$1"
  shift
  if "$@" >/dev/null 2>&1; then
    fail "$description unexpectedly passed"
  fi
  echo "PASS: $description"
}

jq '.vm_median_ms = 149.833' \
  "$REPO_ROOT/benchmarks/baseline/fib.json" > "$TMP/fib-slow.json"
jq '.vm_median_ms = 90.0' \
  "$REPO_ROOT/benchmarks/baseline/fib.json" > "$TMP/fib-fast.json"
jq '.vm_counter_loop_median_ms = 199.0' \
  "$REPO_ROOT/benchmarks/baseline/extended.json" > "$TMP/extended-slow.json"
jq '.vm_counter_loop_median_ms = 150.0' \
  "$REPO_ROOT/benchmarks/baseline/extended.json" > "$TMP/extended-fast.json"
jq '.vm_counter_loop_median_ms = 200.404' \
  "$REPO_ROOT/benchmarks/baseline/extended.json" > "$TMP/extended-drift.json"
jq '.vm_counter_loop_median_ms = 220.0' \
  "$REPO_ROOT/benchmarks/baseline/extended.json" > "$TMP/extended-regression.json"
jq 'del(.vm_median_ms)' \
  "$REPO_ROOT/benchmarks/baseline/fib.json" > "$TMP/fib-missing.json"
jq 'del(.vm_counter_loop_median_ms)' \
  "$REPO_ROOT/benchmarks/baseline/extended.json" > "$TMP/extended-missing.json"

expect_fail "fib rejects a single slow candidate" \
  "$REPO_ROOT/scripts/compare_perf.sh" \
  "$REPO_ROOT/benchmarks/baseline/fib.json" "$TMP/fib-slow.json"
expect_pass "fib selects a passing candidate" \
  "$REPO_ROOT/scripts/compare_perf.sh" \
  "$REPO_ROOT/benchmarks/baseline/fib.json" "$TMP/fib-slow.json" "$TMP/fib-fast.json"

expect_fail "extended rejects a single slow candidate" \
  "$REPO_ROOT/benchmarks/extended/compare.sh" \
  "$REPO_ROOT/benchmarks/baseline/extended.json" "$TMP/extended-slow.json"
expect_pass "extended selects a passing candidate" \
  "$REPO_ROOT/benchmarks/extended/compare.sh" \
  "$REPO_ROOT/benchmarks/baseline/extended.json" "$TMP/extended-slow.json" "$TMP/extended-fast.json"
expect_pass "extended accepts observed counter-loop drift under the CI budget" \
  env PERF_THRESHOLD_PCT_VM_COUNTER=30 \
  "$REPO_ROOT/benchmarks/extended/compare.sh" \
  "$REPO_ROOT/benchmarks/baseline/extended.json" "$TMP/extended-drift.json"
expect_fail "extended rejects counter-loop regression above the CI budget" \
  env PERF_THRESHOLD_PCT_VM_COUNTER=30 \
  "$REPO_ROOT/benchmarks/extended/compare.sh" \
  "$REPO_ROOT/benchmarks/baseline/extended.json" "$TMP/extended-regression.json"

expect_fail "fib rejects missing measurements" \
  "$REPO_ROOT/scripts/compare_perf.sh" \
  "$REPO_ROOT/benchmarks/baseline/fib.json" "$TMP/fib-missing.json"
expect_fail "extended rejects missing measurements" \
  "$REPO_ROOT/benchmarks/extended/compare.sh" \
  "$REPO_ROOT/benchmarks/baseline/extended.json" "$TMP/extended-missing.json"

echo "PASS: test-compare-perf.sh"
