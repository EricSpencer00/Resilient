#!/usr/bin/env bash
# release-claims.sh — release all file claims for a branch (call on PR close)
#
# Usage:
#   agent-scripts/release-claims.sh <branch> [<pr_number>]
#
# Wire this up as a GitHub Actions step on PR close, or call manually.
#
# RES-3976: claims live on the dedicated `agent-claims` ref (see
# claims-ref.sh), not in a file committed on `main`. Releasing no longer
# means opening a "chore: release file claims" PR against `main` (that PR
# was itself the thing that staled every other open PR's copy of
# file-claims.json) — it's a direct CAS push to the claims ref, same as
# claim-files.sh.

set -euo pipefail

REPO_ROOT="$(git rev-parse --show-toplevel)"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=/dev/null
source "$SCRIPT_DIR/claims-ref.sh"

BRANCH="${1:-$(git rev-parse --abbrev-ref HEAD)}"
PR_NUMBER="${2:-}"

cd "$REPO_ROOT"

# RES-1260: sweep stale claims (branch no longer on the remote) in the same
# push as the specific-branch release, so a merge always leaves the ref
# self-healed even if a prior sweep run was skipped or failed.
REMOTE_BRANCHES_FILE="$(mktemp)"
trap 'rm -f "$REMOTE_BRANCHES_FILE"' EXIT
if git ls-remote --heads "$(claims_remote_name)" 2>/dev/null \
  | awk '{print $2}' | sed 's|^refs/heads/||' \
  > "$REMOTE_BRANCHES_FILE"; then
  SWEEP_OK=1
else
  SWEEP_OK=0
fi

edit_release() {
  local claims_file="$1"
  python3 - "$claims_file" "$BRANCH" "$REMOTE_BRANCHES_FILE" "$SWEEP_OK" <<'PYEOF'
import sys, json

claims_file, branch, remote_file, sweep_ok = sys.argv[1:5]

with open(claims_file) as f:
    data = json.load(f)

claims = data.get("claims", {})
released = [f for f, v in claims.items() if v.get("branch") == branch]
for f in released:
    del claims[f]

# RES-1260: self-healing sweep — drop any remaining claim whose branch no
# longer exists on the remote at all (covers claims from earlier merges
# that a failed workflow run left behind).
swept = []
if sweep_ok == "1":
    with open(remote_file) as f:
        live = {line.strip() for line in f if line.strip()}
    swept = [
        path for path, claim in list(claims.items())
        if claim.get("branch") not in live
    ]
    for path in swept:
        del claims[path]

with open(claims_file, "w") as f:
    json.dump(data, f, indent=2)

summary_lines = []
if released:
    summary_lines.append(f"Released {len(released)} claim(s) for {branch}:")
    for f in released:
        summary_lines.append(f"  {f}")
else:
    summary_lines.append(f"No claims found for {branch}.")
if swept:
    summary_lines.append(f"Swept {len(swept)} additional stale claim(s):")
    for f in swept:
        summary_lines.append(f"  {f}")

with open(claims_file + ".summary", "w") as f:
    f.write("\n".join(summary_lines) + "\n")
PYEOF
}

claims_apply_with_retry edit_release "release: ${BRANCH} (PR #${PR_NUMBER:-unknown})"

REPO_SLUG="$(gh repo view --json nameWithOwner -q .nameWithOwner)"

if [ -z "$PR_NUMBER" ]; then
  PR_NUMBER="$(gh pr list --state merged --head "$BRANCH" --json number --jq '.[0].number // ""' 2>/dev/null || true)"
fi

if [ -z "$PR_NUMBER" ] || [ "$PR_NUMBER" = "null" ]; then
  echo "No merged PR found for branch ${BRANCH}; skipping linked-issue closure."
  exit 0
fi

echo "Resolved merged PR #${PR_NUMBER} for branch ${BRANCH}."

PR_STATE="$(gh pr view "$PR_NUMBER" --json state -q .state 2>/dev/null || echo "UNKNOWN")"
PR_MERGED_AT="$(gh pr view "$PR_NUMBER" --json mergedAt -q '.mergedAt // ""' 2>/dev/null || true)"
if [ "$PR_STATE" != "MERGED" ] && [ -z "$PR_MERGED_AT" ]; then
  echo "PR #${PR_NUMBER} is ${PR_STATE}; claims released but linked-issue closure skipped."
  exit 0
fi

if ! gh pr view "$PR_NUMBER" --json labels -q '.labels[].name' | grep -Fxq "agent-vetted"; then
  echo "PR #${PR_NUMBER} is not agent-vetted; claims released but linked-issue closure skipped."
  gh pr comment "$PR_NUMBER" --body "Linked issue closure skipped because this merged PR was not marked \`agent-vetted\` by \`agent-scripts/ready-or-bail.sh\`." >/dev/null || true
  exit 0
fi

CLOSING_ISSUES="$(python3 - "$PR_NUMBER" "$REPO_SLUG" <<'PYEOF'
import json
import os
import subprocess
import sys

pr_number = sys.argv[1]
repo_owner, repo_name = sys.argv[2].split("/", 1)

raw = subprocess.check_output(
    ["gh", "pr", "view", pr_number, "--json", "closingIssuesReferences"],
    text=True,
)
payload = json.loads(raw)
refs = payload.get("closingIssuesReferences") or []

seen = []
for ref in refs:
    repository = ref.get("repository") or {}
    owner = repository.get("owner", {}).get("login")
    name = repository.get("name")
    if owner != repo_owner or name != repo_name:
        continue
    issue = str(ref.get("number", "")).strip()
    if issue and issue not in seen:
        seen.append(issue)

print("\n".join(seen))
PYEOF
)"

if [ -z "$CLOSING_ISSUES" ]; then
  echo "No linked issues for PR #${PR_NUMBER}; nothing to close."
  exit 0
fi

while IFS= read -r issue; do
  [ -z "$issue" ] && continue
  state="$(gh issue view "$issue" --json state -q .state || echo "UNKNOWN")"
  if [ "$state" = "OPEN" ]; then
    if gh issue close "$issue" --reason completed --comment "Closed automatically because PR #${PR_NUMBER} was merged."; then
      post_state="$(gh issue view "$issue" --json state -q .state || echo "UNKNOWN")"
      if [ "$post_state" = "OPEN" ]; then
        echo "WARNING: issue #${issue} remained open after the close attempt for PR #${PR_NUMBER}."
        if [ -n "$PR_NUMBER" ]; then
          gh pr comment "$PR_NUMBER" --body "Linked issue #${issue} remained open after the merge workflow attempted to close it. Please close it manually." >/dev/null || true
        fi
      else
        echo "Closed linked issue #${issue} for PR #${PR_NUMBER}."
      fi
      continue
    fi
    echo "WARNING: failed to close linked issue #${issue} for PR #${PR_NUMBER}."
    if [ -n "$PR_NUMBER" ]; then
      gh pr comment "$PR_NUMBER" --body "Linked issue #${issue} could not be closed automatically after merge. Please close it manually." >/dev/null || true
    fi
  else
    echo "Issue #${issue} is already ${state}; no action."
  fi
done <<< "$CLOSING_ISSUES"
