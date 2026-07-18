#!/usr/bin/env bash
# RES-4125 regression test: agent-scripts/filter-required-checks.sh must
# ignore stale check runs superseded by a newer run of the same name,
# while still blocking on a genuinely-latest non-SUCCESS run.
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")"

fail=0

check() {
  local desc="$1" input="$2" expected="$3" actual
  actual=$(echo "$input" | ./filter-required-checks.sh)
  if [[ "$actual" != "$expected" ]]; then
    echo "FAIL: $desc"
    echo "  expected: [$expected]"
    echo "  actual:   [$actual]"
    fail=1
  else
    echo "PASS: $desc"
  fi
}

# PR #4124 scenario: a draft-era CANCELLED run of "build / test / clippy"
# is superseded by a later SUCCESS run of the same name on the same SHA.
# The stale CANCELLED run must not block.
check "stale CANCELLED superseded by later SUCCESS is ignored" '
[
  {"name": "build / test / clippy", "conclusion": "CANCELLED", "startedAt": "2026-07-18T10:00:00Z", "completedAt": "2026-07-18T10:01:00Z"},
  {"name": "build / test / clippy", "conclusion": "SUCCESS", "startedAt": "2026-07-18T11:00:00Z", "completedAt": "2026-07-18T11:05:00Z"}
]' ''

# If the latest run of a required check is itself CANCELLED, it must
# still block — the gate is not loosened.
check "latest run CANCELLED still blocks" '
[
  {"name": "build / test / clippy", "conclusion": "SUCCESS", "startedAt": "2026-07-18T10:00:00Z", "completedAt": "2026-07-18T10:01:00Z"},
  {"name": "build / test / clippy", "conclusion": "CANCELLED", "startedAt": "2026-07-18T11:00:00Z", "completedAt": "2026-07-18T11:05:00Z"}
]' 'build / test / clippy=CANCELLED'

# diff-shape guardrail overlap sub-check is allowed to be FAILURE
# regardless of recency.
check "diff-shape guardrail FAILURE is excluded" '
[
  {"name": "diff-shape guardrail", "conclusion": "FAILURE", "startedAt": "2026-07-18T11:00:00Z", "completedAt": "2026-07-18T11:05:00Z"},
  {"name": "build / test / clippy", "conclusion": "SUCCESS", "startedAt": "2026-07-18T10:00:00Z", "completedAt": "2026-07-18T10:01:00Z"}
]' ''

# fib(25) medians flake is excluded regardless of recency.
check "fib(25) medians FAILURE is excluded" '
[
  {"name": "fib(25) medians", "conclusion": "FAILURE", "startedAt": "2026-07-18T11:00:00Z", "completedAt": "2026-07-18T11:05:00Z"},
  {"name": "build / test / clippy", "conclusion": "SUCCESS", "startedAt": "2026-07-18T10:00:00Z", "completedAt": "2026-07-18T10:01:00Z"}
]' ''

# A required check still IN_PROGRESS (no completedAt, null conclusion)
# must still block, and compares fine against a completed run.
check "in-progress (null conclusion) run blocks" '
[
  {"name": "board hygiene", "conclusion": null, "startedAt": "2026-07-18T11:00:00Z", "completedAt": null}
]' 'board hygiene=PENDING'

# Multiple distinct required checks all clean -> empty output.
check "all distinct checks green" '
[
  {"name": "build / test / clippy", "conclusion": "SUCCESS", "startedAt": "2026-07-18T10:00:00Z", "completedAt": "2026-07-18T10:01:00Z"},
  {"name": "board hygiene", "conclusion": "SUCCESS", "startedAt": "2026-07-18T10:00:00Z", "completedAt": "2026-07-18T10:01:00Z"}
]' ''

if [[ "$fail" -ne 0 ]]; then
  echo "test-filter-required-checks.sh: FAILED"
  exit 1
fi
echo "test-filter-required-checks.sh: all passed"
