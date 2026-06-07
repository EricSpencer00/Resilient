#!/usr/bin/env bash
# release-claims.sh — release all file claims for a branch (call on PR close)
#
# Usage:
#   .claude/scripts/release-claims.sh <branch> [<pr_number>]
#
# Wire this up as a GitHub Actions step on PR close, or call manually.

set -euo pipefail

REPO_ROOT="$(git rev-parse --show-toplevel)"
CLAIMS_FILE="$REPO_ROOT/agent-scripts/file-claims.json"
BRANCH="${1:-$(git rev-parse --abbrev-ref HEAD)}"
PR_NUMBER="${2:-}"

if [ ! -f "$CLAIMS_FILE" ]; then
  echo "No claims file found — nothing to release."
  exit 0
fi

REPO_SLUG="$(gh repo view --json nameWithOwner -q .nameWithOwner)"
python3 - "$CLAIMS_FILE" "$BRANCH" <<'PYEOF'
import sys, json

claims_file, branch = sys.argv[1], sys.argv[2]

with open(claims_file) as f:
    data = json.load(f)

claims = data.get("claims", {})
released = [f for f, v in claims.items() if v["branch"] == branch]

for f in released:
    del claims[f]

with open(claims_file, "w") as f:
    json.dump(data, f, indent=2)

if released:
    print(f"Released {len(released)} claim(s) for {branch}:")
    for f in released:
        print(f"  {f}")
else:
    print(f"No claims found for {branch}.")
PYEOF

if [ -z "$PR_NUMBER" ]; then
  PR_NUMBER="$(gh pr list --state merged --head "$BRANCH" --json number --jq '.[0].number // ""' 2>/dev/null || true)"
fi

if [ -z "$PR_NUMBER" ] || [ "$PR_NUMBER" = "null" ]; then
  echo "No merged PR found for branch ${BRANCH}; skipping linked-issue closure."
  exit 0
fi

echo "Resolved merged PR #${PR_NUMBER} for branch ${BRANCH}."

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
      echo "Closed linked issue #${issue} for PR #${PR_NUMBER}."
      continue
    fi
  else
    echo "Issue #${issue} is already ${state}; no action."
  fi
done <<< "$CLOSING_ISSUES"
