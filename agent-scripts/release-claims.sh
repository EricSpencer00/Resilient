#!/usr/bin/env bash
# release-claims.sh — release all file claims for a branch (call on PR merge)
#
# Usage:
#   .claude/scripts/release-claims.sh <branch>
#
# Wire this up as a GitHub Actions step on PR merge, or call manually.

set -euo pipefail

REPO_ROOT="$(git rev-parse --show-toplevel)"
CLAIMS_FILE="$REPO_ROOT/agent-scripts/file-claims.json"
BRANCH="${1:-$(git rev-parse --abbrev-ref HEAD)}"

if [ ! -f "$CLAIMS_FILE" ]; then
  echo "No claims file found — nothing to release."
  exit 0
fi

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
