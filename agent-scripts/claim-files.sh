#!/usr/bin/env bash
# claim-files.sh — register file ownership for an agent branch
#
# Usage: agent-scripts/claim-files.sh <branch> <file1> [file2 ...]
#
# Agents MUST call this immediately after creating their branch,
# before modifying any core files. The claim prevents other agents
# from being dispatched to the same files.
#
# RES-2670: as a side-effect, every claim sweeps the file for stale
# entries whose branch no longer exists on `origin`. The bot-driven
# `release-file-claims` workflow has been failing for days because
# branch protection rejects its direct push to `main` (the chore
# commit cannot satisfy required status checks). Local sweep here
# is the primary cleanup mechanism — the workflow remains as a
# best-effort safety net.

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

# RES-2670: list every branch currently on the remote so the sweep
# below can decide which claims are stale. Cache it in a temp file
# rather than passing through python's environment so paths with
# slashes/colons survive intact. A failed `ls-remote` (e.g. offline,
# auth blip) is non-fatal — we skip the sweep rather than risk
# deleting live claims.
REMOTE_BRANCHES_FILE="$(mktemp)"
trap 'rm -f "$REMOTE_BRANCHES_FILE"' EXIT
if git ls-remote --heads origin 2>/dev/null \
  | awk '{print $2}' | sed 's|^refs/heads/||' \
  > "$REMOTE_BRANCHES_FILE"; then
  SWEEP_OK=1
else
  SWEEP_OK=0
fi

TIMESTAMP=$(date -u +"%Y-%m-%dT%H:%M:%SZ")

python3 - "$CLAIMS_FILE" "$BRANCH" "$TIMESTAMP" "$REMOTE_BRANCHES_FILE" "$SWEEP_OK" "${FILES[@]}" <<'PYEOF'
import sys, json

claims_file, branch, timestamp, remote_file, sweep_ok = sys.argv[1:6]
files = sys.argv[6:]

with open(claims_file) as f:
    data = json.load(f)

claims = data.setdefault("claims", {})

# RES-2670: drop claims whose branch is no longer on the remote
# (merged & deleted, abandoned, etc.) so a failed release-claims
# workflow run does not leave the file blocked for future agents.
# The current branch is always considered live — even if `gh` hasn't
# heard about it yet (first claim before first push).
swept = []
if sweep_ok == "1":
    with open(remote_file) as f:
        live = {line.strip() for line in f if line.strip()}
    live.add(branch)
    swept = [
        path for path, claim in list(claims.items())
        if claim.get("branch") not in live
    ]
    for path in swept:
        del claims[path]

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

if swept:
    print(f"Swept {len(swept)} stale claim(s):")
    for path in swept:
        print(f"  {path}")
print(f"Claimed {len(files)} file(s) for {branch}:")
for f in files:
    print(f"  {f}")
PYEOF
