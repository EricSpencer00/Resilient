#!/usr/bin/env bash
# check-overlaps.sh — pre-dispatch safety check for parallel agents
#
# Usage:
#   agent-scripts/check-overlaps.sh <file1> [file2 ...]
#   agent-scripts/check-overlaps.sh --pr-files <branch>
#
# Returns:
#   0  — no conflicts, safe to dispatch
#   1  — conflicts found, printed to stdout
#
# Examples:
#   agent-scripts/check-overlaps.sh resilient/src/main.rs resilient/src/typechecker.rs
#   agent-scripts/check-overlaps.sh --pr-files res-NNN-my-feature

set -euo pipefail

REPO_ROOT="$(git rev-parse --show-toplevel)"
CLAIMS_FILE="$REPO_ROOT/agent-scripts/file-claims.json"

if [ ! -f "$CLAIMS_FILE" ]; then
  echo '{"claims":{}}' > "$CLAIMS_FILE"
fi

FILES_TO_CHECK=()
CHECK_MODE="files"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --pr-files)
      CHECK_MODE="pr-files"
      BRANCH="${2:-}"
      shift 2
      ;;
    *)
      FILES_TO_CHECK+=("$1")
      shift
      ;;
  esac
done

if [ "$CHECK_MODE" = "pr-files" ]; then
  if [ -z "${BRANCH:-}" ]; then
    echo "ERROR: --pr-files requires a branch name" >&2
    exit 1
  fi
  mapfile -t FILES_TO_CHECK < <(git diff --name-only "origin/main...${BRANCH}" 2>/dev/null || true)
fi

if [ ${#FILES_TO_CHECK[@]} -eq 0 ]; then
  echo "No files to check."
  exit 0
fi

CONFLICTS_FOUND=0

echo "Checking ${#FILES_TO_CHECK[@]} file(s) against open PRs and claims..."
echo

# When run as `--pr-files <branch>`, the file list IS that PR's diff —
# matching it back against the same PR is always a self-match, never a
# real conflict. SELF_BRANCH lets us filter the current PR out of the
# overlap check below.
SELF_BRANCH=""
if [ "$CHECK_MODE" = "pr-files" ]; then
  SELF_BRANCH="$BRANCH"
fi

# 1. Check GitHub open PRs
OPEN_PR_BRANCHES=""
if command -v gh &>/dev/null; then
  PR_JSON=$(gh pr list --state open --json number,headRefName,files 2>/dev/null || echo "[]")
  # Cache the set of currently-open PR branches so the claims pass below
  # can spot stale claims (claim points at a branch with no open PR).
  OPEN_PR_BRANCHES=$(echo "$PR_JSON" | python3 -c "
import sys, json
data = json.load(sys.stdin)
print('\n'.join(sorted({pr['headRefName'] for pr in data})))
" 2>/dev/null || true)

  for file in "${FILES_TO_CHECK[@]}"; do
    MATCHING=$(echo "$PR_JSON" | python3 -c "
import sys, json
data = json.load(sys.stdin)
target = sys.argv[1]
self_branch = sys.argv[2] if len(sys.argv) > 2 else ''
for pr in data:
    if pr.get('headRefName') == self_branch:
        continue
    for f in pr.get('files', []):
        if f['path'] == target:
            print(f'PR #{pr[\"number\"]} ({pr[\"headRefName\"]})')
            break
" "$file" "$SELF_BRANCH" 2>/dev/null || true)
    if [ -n "$MATCHING" ]; then
      echo "  CONFLICT  $file"
      echo "$MATCHING" | while read -r line; do echo "            ↳ $line"; done
      CONFLICTS_FOUND=1
    fi
  done
fi

# 2. Check local claims registry. Claims whose branch isn't in the
# current `gh pr list --state open` are treated as stale — they
# surface as info but don't fail the check (the claim file will be
# auto-cleaned when the merge workflow runs release-claims.sh, but
# we don't want stale claims from already-merged branches blocking
# unrelated work indefinitely).
python3 - "$CLAIMS_FILE" "$SELF_BRANCH" "$OPEN_PR_BRANCHES" "${FILES_TO_CHECK[@]}" <<'PYEOF'
import sys, json

claims_file = sys.argv[1]
self_branch = sys.argv[2]
open_branches = set(filter(None, sys.argv[3].splitlines()))
files_to_check = sys.argv[4:]
found = False

with open(claims_file) as f:
    data = json.load(f)

for file in files_to_check:
    c = data.get("claims", {}).get(file)
    if not c:
        continue
    branch = c["branch"]
    # Self-claims aren't conflicts.
    if branch == self_branch:
        continue
    # If we have a list of open PR branches, treat claims from
    # branches with no open PR as stale (info only).
    if open_branches and branch not in open_branches:
        print(f"  stale claim (no open PR): {file}")
        print(f"            ↳ branch: {branch} (since {c['claimed_at']}); will not block")
        continue
    print(f"  CLAIMED   {file}")
    print(f"            ↳ branch: {branch} (since {c['claimed_at']})")
    found = True

sys.exit(1 if found else 0)
PYEOF
CLAIMS_EXIT=$?
[ $CLAIMS_EXIT -ne 0 ] && CONFLICTS_FOUND=1

echo
if [ $CONFLICTS_FOUND -eq 0 ]; then
  echo "✅  No conflicts. Safe to dispatch."
else
  echo "⚠️   Conflicts detected — wait for those PRs to merge, or use the feature isolation pattern:"
  echo "     Create resilient/src/your_feature.rs for ALL logic."
  echo "     Only touch EXTENSION_TOKENS / EXTENSION_KEYWORDS / EXTENSION_PASSES blocks."
  echo "     See CLAUDE.md §feature-isolation-pattern"
fi

exit $CONFLICTS_FOUND
