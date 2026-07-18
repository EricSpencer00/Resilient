#!/usr/bin/env bash
# RES-4125: Dedupe check runs by name, considering only the latest run
# per name, before evaluating pass/fail.
#
# Why: a PR's statusCheckRollup can contain multiple runs of the same
# check name on one head SHA — e.g. a CANCELLED run from a draft-era
# push, superseded by a later SUCCESS run once the PR went ready. The
# old evaluation looked at every entry, so a stale CANCELLED run
# permanently blocked auto-merge even though the current run of that
# check was green (observed on PR #4124).
#
# Usage:
#   echo "$statusCheckRollup_json_array" | agent-scripts/filter-required-checks.sh
#
# Input: JSON array as produced by
#   gh pr view --json statusCheckRollup --jq '.statusCheckRollup'
#
# Output: a " | "-joined "name=conclusion" list of checks that are not
# clean (excluding `diff-shape guardrail` and `fib(25) medians`, which
# are allowed to be non-SUCCESS per branch protection). Empty output
# means every required check's latest run is SUCCESS.
set -euo pipefail

jq -r '
  group_by(.name)
  | map(max_by(.completedAt // .startedAt // ""))
  | [.[]
      | select(.name | test("diff-shape guardrail") | not)
      | select(.name | test("fib\\(25\\) medians") | not)
      | select(.conclusion == "FAILURE" or .conclusion == "CANCELLED" or .conclusion == "TIMED_OUT" or .conclusion == null)
      | (.name + "=" + (.conclusion // "PENDING"))
    ]
  | join(" | ")
'
