#!/usr/bin/env bash
# refresh-open-prs.sh — update every open non-draft PR branch against main.
#
# Intended to run after pushes to `main` so in-flight work keeps moving
# without waiting for a human to notice the branch is stale.
#
# Behavior:
#   - skips draft PRs
#   - skips PRs whose base is not `main`
#   - uses `gh pr update-branch --rebase`
#   - comments on PRs whose refresh attempt failed so the blockage is visible
#   - respects COMMENT_ON_FAILURE=0 for quiet safety sweeps
#
# Usage:
#   agent-scripts/refresh-open-prs.sh

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(git rev-parse --show-toplevel)"
cd "$REPO_ROOT"

export GH_TOKEN="${GH_TOKEN:-}"
export COMMENT_ON_FAILURE="${COMMENT_ON_FAILURE:-1}"

PRS_JSON="$(gh pr list --state open --base main --limit 100 \
  --json number,title,isDraft,baseRefName,headRefName,updatedAt 2>/dev/null || echo '[]')"

PRS_JSON="$PRS_JSON" python3 <<'PYEOF'
import json
import os
import subprocess
import sys

prs = json.loads(os.environ.get("PRS_JSON") or "[]")
if not prs:
    print("No open PRs targeting main.")
    sys.exit(0)

updated = 0
skipped = 0
failed = 0

for pr in prs:
    number = pr["number"]
    title = pr.get("title") or ""
    if pr.get("isDraft"):
        print(f"skip #{number}: draft")
        skipped += 1
        continue
    if pr.get("baseRefName") != "main":
        print(f"skip #{number}: base={pr.get('baseRefName')}")
        skipped += 1
        continue

    print(f"refreshing #{number} — {title[:80]}")
    r = subprocess.run(
        ["gh", "pr", "update-branch", str(number), "--rebase"],
        text=True,
        capture_output=True,
    )
    if r.returncode == 0:
        updated += 1
        print(f"  updated #{number}")
        continue

    failed += 1
    msg = (r.stdout + "\n" + r.stderr).strip()
    print(f"  update failed for #{number}")
    if msg:
        for line in msg.splitlines()[-5:]:
            print(f"    {line}")

    if os.environ.get("COMMENT_ON_FAILURE", "1") != "0":
        # Comment once per failed refresh attempt so the blockage is
        # visible in the PR timeline. This is intentionally terse.
        body = (
            "Automatic refresh after a push to `main` could not update this branch.\n\n"
            "The branch is now stale relative to `main`, so auto-merge cannot resume "
            "until it is rebased or the conflict is resolved."
        )
        comment = subprocess.run(
            ["gh", "pr", "comment", str(number), "--body", body],
            text=True,
            capture_output=True,
        )
        if comment.returncode != 0:
            print("    warning: could not post PR comment")
    else:
        print("    failure comment suppressed for this run")

print(f"Summary: updated={updated} skipped={skipped} failed={failed}")
sys.exit(0)
PYEOF
