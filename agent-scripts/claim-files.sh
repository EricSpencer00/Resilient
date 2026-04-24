#!/usr/bin/env bash
# claim-files.sh — register file ownership for an agent branch
#
# Usage: agent-scripts/claim-files.sh <branch> <file1> [file2 ...]
#
# Agents MUST call this immediately after creating their branch,
# before modifying any core files. The claim prevents other agents
# from being dispatched to the same files.
#
# Complementary script: release-claims.sh (called on PR merge)

set -euo pipefail

REPO_ROOT="$(git rev-parse --show-toplevel)"
CLAIMS_FILE="$REPO_ROOT/agent-scripts/file-claims.json"
BRANCH="${1:-}"

if [ -z "$BRANCH" ]; then
  echo "Usage: claim-files.sh <branch> <file1> [file2 ...]" >&2
  exit 1
fi

shift
FILES=("$@")

if [ ${#FILES[@]} -eq 0 ]; then
  echo "Usage: claim-files.sh <branch> <file1> [file2 ...]" >&2
  exit 1
fi

# Ensure claims file exists
if [ ! -f "$CLAIMS_FILE" ]; then
  echo '{"claims":{}}' > "$CLAIMS_FILE"
fi

TIMESTAMP=$(date -u +"%Y-%m-%dT%H:%M:%SZ")

python3 - "$CLAIMS_FILE" "$BRANCH" "$TIMESTAMP" "${FILES[@]}" <<'PYEOF'
import sys, json

claims_file, branch, timestamp = sys.argv[1], sys.argv[2], sys.argv[3]
files = sys.argv[4:]

with open(claims_file) as f:
    data = json.load(f)

claims = data.setdefault("claims", {})
conflicts = []

for file in files:
    if file in claims and claims[file]["branch"] != branch:
        conflicts.append(f"  {file} → already claimed by {claims[file]['branch']}")

if conflicts:
    print("ERROR: Cannot claim files already owned by other branches:")
    for c in conflicts:
        print(c)
    sys.exit(1)

for file in files:
    claims[file] = {"branch": branch, "claimed_at": timestamp}

with open(claims_file, "w") as f:
    json.dump(data, f, indent=2)

print(f"Claimed {len(files)} file(s) for {branch}:")
for f in files:
    print(f"  {f}")
PYEOF
