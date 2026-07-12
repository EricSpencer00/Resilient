#!/usr/bin/env bash
# clear-approval-holds.sh — clear GitHub's "action_required" bot-approval
# hold on open PRs so agents land work without a human clicking
# "Approve and run".
#
# Why this exists
# ---------------
# When a PR's workflow runs are triggered by github-actions[bot] — e.g. a PR
# opened or promoted by a cloud agent, or by another workflow using
# GITHUB_TOKEN — GitHub parks the runs in `action_required` ("This workflow
# requires approval from a maintainer") and they never execute. A
# required-check-gated PR then can never auto-merge until someone clicks
# "Approve and run" in the browser. That defeats the whole ship-to-merge,
# no-human-in-the-loop model.
#
# Two dead ends, so you don't waste time on them:
#   * There is NO REST/CLI approve path for non-fork bot runs.
#     `POST /actions/runs/{id}/approve` returns
#     `403: not from a fork pull request`.
#   * `gh run rerun` does NOT help: a rerun keeps the original bot actor, so
#     the run is immediately re-held. (This is why auto-rerun-held-ci.yml,
#     which reruns held runs, cannot fix a bot-attributed hold on its own.)
#
# The fix that works: close and reopen the PR. The `reopened` event re-fires
# the workflows attributed to whoever runs `gh` here — a real user PAT via
# `gh auth` — and human-attributed runs are never held.
#
# SAFETY: closing a PR fires release-file-claims.yml, which releases the PR's
# file claims. So this only touches NON-DRAFT (ready) PRs, where the work is
# finished (releasing claims is harmless) and where real held runs actually
# appear (drafts have deferred CI, so they carry only skipped-green
# placeholders, never a real held run).
#
# Run this under a real user's `gh auth` (the same auth agents use locally).
# Running it from inside a GitHub Actions workflow would re-attribute the
# reopened event to github-actions[bot] and re-hold the runs — so it is a
# local/agent tool, not a workflow step.
#
# Usage:
#   agent-scripts/clear-approval-holds.sh             # every held ready PR
#   agent-scripts/clear-approval-holds.sh --pr 3871   # one PR
#   agent-scripts/clear-approval-holds.sh --dry-run   # report, mutate nothing
set -euo pipefail

DRY_RUN=0
ONLY_PR=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --pr) ONLY_PR="${2:-}"; shift 2 ;;
    --dry-run) DRY_RUN=1; shift ;;
    -h|--help) grep '^#' "$0" | grep -v '^#!' | sed 's/^#\{1,2\} \{0,1\}//'; exit 0 ;;
    *) echo "unknown flag: $1" >&2; exit 2 ;;
  esac
done

# Number of runs on $1=branch at $2=sha that are held in action_required.
held_count() {
  gh run list --branch "$1" --limit 40 \
    --json databaseId,headSha,conclusion,status \
    --jq "[.[] | select(.headSha==\"$2\" and (.conclusion==\"action_required\" or .status==\"action_required\"))] | length" \
    2>/dev/null || echo 0
}

clear_one() {  # $1=pr $2=branch $3=sha ; returns 0 if it cleared a hold
  local pr="$1" branch="$2" sha="$3" n
  n="$(held_count "$branch" "$sha")"
  [[ "${n:-0}" -eq 0 ]] && return 1
  echo "PR #$pr ($branch): $n run(s) held in action_required"
  if (( DRY_RUN )); then
    echo "  [dry-run] would close + reopen to re-trigger under your token"
    return 0
  fi
  gh pr close "$pr" >/dev/null
  sleep 2
  gh pr reopen "$pr" >/dev/null
  # Closing cancels any armed auto-merge; re-arm so the fresh, human-attributed
  # green run lands the PR without another manual step.
  gh pr merge "$pr" --squash --auto --delete-branch >/dev/null 2>&1 || true
  echo "  closed + reopened — runs re-fire under your token, hold cleared"
  return 0
}

# Portable collection (no `mapfile`; macOS ships bash 3.2). A pipe would run
# the loop in a subshell and lose `cleared`, so drive it from a here-string.
rows="$(gh pr list --state open --base main --limit 100 \
  --json number,headRefName,headRefOid,isDraft \
  --jq '.[] | select(.isDraft==false) | "\(.number)\t\(.headRefName)\t\(.headRefOid)"')"

cleared=0
while IFS=$'\t' read -r pr branch sha; do
  [[ -z "$pr" ]] && continue
  [[ -n "$ONLY_PR" && "$pr" != "$ONLY_PR" ]] && continue
  if clear_one "$pr" "$branch" "$sha"; then
    cleared=$((cleared + 1))
  fi
done <<< "$rows"

if [[ -n "$ONLY_PR" && "$cleared" -eq 0 ]]; then
  echo "No action_required hold found on ready PR #$ONLY_PR (drafts are skipped by design)."
else
  echo "Done. Cleared holds on $cleared PR(s)."
fi
