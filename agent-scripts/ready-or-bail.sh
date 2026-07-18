#!/usr/bin/env bash
# ready-or-bail.sh — run verify-scope.sh, then either mark the draft PR
# ready for review (green) or post a failure comment and leave it draft.
#
# Usage:
#   agent-scripts/ready-or-bail.sh              # infer PR from current branch
#   agent-scripts/ready-or-bail.sh --pr 232
#   agent-scripts/ready-or-bail.sh --dry-run    # skip gh mutations
#   agent-scripts/ready-or-bail.sh --no-close   # never touch the PR body's
#                                                # Closes/Refs lines at all
#
# This is the ONLY path the orchestrator uses to transition a draft → ready.
# If this script isn't run, the PR stays draft — that's the whole point.
#
# RES-4136: this script never auto-derives `Closes #N` from a `Refs #N`
# line — only an explicit `Closes #N` already in the PR body closes
# anything on merge. See compute_close_issue() below for the history.
# `--no-close` is a belt-and-suspenders opt-out for callers that want a
# guarantee the body is left untouched regardless.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# RES-4021: hardcoded denylist of tracker/umbrella issue numbers that must
# NEVER be auto-closed by the Refs/Closes heuristic below, even when they're
# the first "#N" mentioned in a PR body's "Refs #N · EPIC" convention line.
# #3933 is the v1.0 roadmap tracker; child PRs reference it via `Refs #3933`
# and must not accidentally close it on merge.
TRACKER_ISSUE_DENYLIST=(3933)

# is_tracker_issue ISSUE — true (0) if ISSUE must never be auto-closed.
# Checks the hardcoded denylist first (works offline, e.g. in tests), then
# falls back to a live `gh issue view` label lookup for issues not on the
# denylist, matching (case-insensitively) the labels tracker/epic/umbrella.
is_tracker_issue() {
  local issue="$1"
  local n
  for n in "${TRACKER_ISSUE_DENYLIST[@]}"; do
    [ "$n" = "$issue" ] && return 0
  done
  local labels
  labels="$(gh issue view "$issue" --json labels -q '.labels[].name' 2>/dev/null || true)"
  printf '%s\n' "$labels" | grep -qiE '^(tracker|epic|umbrella)$'
}

# compute_close_issue BODY_FILE [IS_TRACKER_FN] — derives which issue
# number, if any, should get a `Closes #N` line appended to BODY_FILE.
# Prints the issue number on stdout, or nothing.
#
# RES-4136: `Refs #N` is NEVER treated as a closing signal. `Refs #N`
# means "this PR is part of the work for ticket N" — it says nothing
# about whether *this* PR is the final increment. Auto-converting it
# into `Closes #N` wrongly closed umbrella/parent issues #4083 and
# #4063, which carried plain `ticket`/`enhancement` labels (not
# tracker/epic/umbrella), so the RES-4021 tracker denylist didn't catch
# them either. The only way an issue closes now is an explicit
# `Closes #N` the agent wrote into the PR body themselves, attesting
# that this PR really does finish ticket N.
#
#   1. If BODY_FILE already has an explicit `Closes #N`, that's the
#      agent's own attestation — print nothing (nothing to append; the
#      explicit line stays as-is and does the closing on merge).
#   2. Otherwise — whether or not a `Refs #N` line is present — print
#      nothing. There is no safe automatic derivation left; the agent
#      must opt in explicitly.
#
# IS_TRACKER_FN is accepted for backward compatibility with existing
# callers/tests but is no longer consulted here, since Refs no longer
# feeds into closing at all. is_tracker_issue() itself is kept as a
# building block in case a future explicit-Closes-derived-from-Refs
# path needs it again.
compute_close_issue() {
  local body_file="$1"
  local _is_tracker_fn="${2:-is_tracker_issue}" # unused: kept for signature compatibility
  : "$_is_tracker_fn"

  local explicit_closes
  explicit_closes="$(sed -nE 's/^[[:space:]]*[Cc]loses[[:space:]]+#([0-9]+).*/\1/p' "$body_file" | head -1 || true)"
  if [ -n "$explicit_closes" ]; then
    return 0
  fi

  return 0
}

# RES-4021: allow this file to be `source`d (e.g. by
# agent-scripts/test-ready-or-bail-closes.sh) to unit-test the functions
# above without running the full draft-to-ready flow, which requires a
# real PR and `gh` mutations.
if [[ "${BASH_SOURCE[0]}" != "${0}" ]]; then
  return 0
fi

REPO_ROOT="$(git rev-parse --show-toplevel)"
cd "$REPO_ROOT"

PR=""
DRY_RUN=0
NO_CLOSE=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --pr) PR="$2"; shift 2 ;;
    --dry-run) DRY_RUN=1; shift ;;
    --no-close) NO_CLOSE=1; shift ;;
    *) echo "unknown flag: $1" >&2; exit 2 ;;
  esac
done

if [ -z "$PR" ]; then
  BRANCH="$(git rev-parse --abbrev-ref HEAD)"
  PR="$(gh pr list --head "$BRANCH" --state open --json number -q '.[0].number' 2>/dev/null || true)"
  if [ -z "$PR" ] || [ "$PR" = "null" ]; then
    echo "Could not infer open PR for branch $BRANCH. Pass --pr N." >&2
    exit 2
  fi
fi

REPORT=/tmp/agent-guardrail-report.json
PRE_SYNC_HEAD="$(git rev-parse HEAD)"

if bash "$SCRIPT_DIR/verify-scope.sh" --report "$REPORT"; then
  echo
  echo "Guardrail green → syncing against agents/integration before marking ready."
  if (( DRY_RUN == 0 )); then
    if ! bash "$SCRIPT_DIR/sync-integration.sh" --pr "$PR"; then
      echo "sync-integration failed — leaving PR #$PR as draft."
      gh pr comment "$PR" --body "Guardrail passed, but \`sync-integration.sh\` failed — conflicts outside the append-only allowlist. Resolve manually, then re-run \`agent-scripts/ready-or-bail.sh\`." >/dev/null
      exit 2
    fi

    POST_SYNC_HEAD="$(git rev-parse HEAD)"
    if [ "$PRE_SYNC_HEAD" != "$POST_SYNC_HEAD" ]; then
      echo "Branch changed while syncing — rerunning guardrail on refreshed HEAD."
      if ! bash "$SCRIPT_DIR/verify-scope.sh" --report "$REPORT"; then
        echo "refreshed guardrail failed after sync — leaving PR #$PR as draft."
        BODY="$(python3 - "$REPORT" <<'PYEOF'
import json, sys
try:
    r = json.load(open(sys.argv[1]))
except Exception:
    print("Guardrail passed before sync, but the branch changed while syncing against `agents/integration` and the refreshed guardrail failed.")
    sys.exit(0)
lines = [
    "Guardrail passed before sync, but the branch changed while syncing against `agents/integration` and the refreshed guardrail failed.",
    "",
    "Violations:",
]
for f in r.get("failures", []):
    lines.append(f"- {f}")
lines += ["", "Fix the items above, push new commits, and re-run `agent-scripts/ready-or-bail.sh`."]
print("\n".join(lines))
PYEOF
)"
        gh pr comment "$PR" --body "$BODY" >/dev/null
        "$SCRIPT_DIR/agent-handoff.sh" \
          --pr "$PR" \
          --phase guardrail-red \
          --status "left draft after refreshed guardrail failure" \
          --summary "$BODY" >/dev/null || true
        exit 1
      fi
  fi

  gh pr ready "$PR" 2>&1 | tail -2
  gh label create "agent-vetted" \
    --color "0E8A16" \
    --description "ready-or-bail passed substantive local guardrails and integration sync" \
    >/dev/null 2>&1 || true
  gh pr edit "$PR" --add-label "agent-vetted" >/dev/null

  BODY_FILE="$(mktemp "${TMPDIR:-/tmp}/resilient-pr-body.XXXXXX")"
  gh pr view "$PR" --json body -q '.body // ""' > "$BODY_FILE"
  echo
  echo "=============================================================="
  if (( NO_CLOSE == 1 )); then
    echo " ISSUE-CLOSE NOTICE: --no-close passed — PR #$PR body will NOT"
    echo " be modified. Whatever Closes/Refs lines are already in the"
    echo " body are the only ones that take effect on merge."
    echo "=============================================================="
  else
    CLOSE_ISSUE="$(compute_close_issue "$BODY_FILE")"
    if [ -n "$CLOSE_ISSUE" ]; then
      echo " ISSUE-CLOSE NOTICE: this should not happen — compute_close_issue"
      echo " returned #$CLOSE_ISSUE. ready-or-bail.sh no longer auto-appends"
      echo " Closes derived from Refs (RES-4136); refusing to append."
      echo "=============================================================="
      CLOSE_ISSUE=""
    else
      EXPLICIT_CLOSES="$(sed -nE 's/^[[:space:]]*[Cc]loses[[:space:]]+#([0-9]+).*/\1/p' "$BODY_FILE" | head -1 || true)"
      if [ -n "$EXPLICIT_CLOSES" ]; then
        echo " ISSUE-CLOSE NOTICE: PR #$PR body already has an explicit"
        echo " 'Closes #$EXPLICIT_CLOSES' — that issue WILL close on merge."
        echo " Nothing appended."
      else
        echo " ISSUE-CLOSE NOTICE: PR #$PR body has no explicit 'Closes #N'."
        echo " NOTHING will close on merge. ready-or-bail.sh never derives"
        echo " Closes from a 'Refs #N' line (RES-4136) — if this PR really"
        echo " is the final increment of its ticket, add an explicit"
        echo " 'Closes #N' line to the PR body yourself before merge."
      fi
      echo "=============================================================="
    fi
  fi
  rm -f "$BODY_FILE" "${BODY_FILE}.next"

  if [ "$PRE_SYNC_HEAD" != "$POST_SYNC_HEAD" ]; then
      READY_BODY="Guardrail passed ✓ — fmt, clippy, tests, diff-shape, overlap. Synced against \`agents/integration\` and rechecked on the refreshed branch. Auto-merge will fire once remaining checks complete."
    else
      READY_BODY="Guardrail passed ✓ — fmt, clippy, tests, diff-shape, overlap. Synced against \`agents/integration\`. Auto-merge will fire once remaining checks complete."
    fi
    gh pr comment "$PR" --body "$READY_BODY" >/dev/null
    "$SCRIPT_DIR/agent-handoff.sh" \
      --pr "$PR" \
      --phase guardrail-green \
      --status "ready for review after local guardrail, integration sync, and freshness recheck" \
      --summary "Local guardrail passed; branch was synced against agents/integration, rechecked if the branch moved, and marked ready." >/dev/null || true
  else
    echo "(dry-run) would also run sync-integration.sh"
  fi
  exit 0
else
  echo
  echo "Guardrail red → leaving PR #$PR as draft and posting failure comment."
  if (( DRY_RUN == 0 )); then
    BODY="$(python3 - "$REPORT" <<'PYEOF'
import json, sys
try:
    r = json.load(open(sys.argv[1]))
except Exception:
    print("Guardrail failed (report missing).")
    sys.exit(0)
lines = ["Guardrail **FAILED** — leaving PR as draft.", "", "Violations:"]
for f in r.get("failures", []):
    lines.append(f"- {f}")
lines += ["", "Fix the items above, push new commits, and re-run `agent-scripts/ready-or-bail.sh`."]
print("\n".join(lines))
PYEOF
)"
    gh pr comment "$PR" --body "$BODY" >/dev/null
    "$SCRIPT_DIR/agent-handoff.sh" \
      --pr "$PR" \
      --phase guardrail-red \
      --status "left draft after guardrail failure" \
      --summary "$BODY" >/dev/null || true
  else
    echo "(dry-run) would comment:"
    cat "$REPORT"
  fi
  exit 1
fi
