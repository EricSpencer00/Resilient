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

# 1. Check GitHub open PRs
if command -v gh &>/dev/null; then
  PR_JSON=$(gh pr list --state open --json number,headRefName,files 2>/dev/null || echo "[]")
  for file in "${FILES_TO_CHECK[@]}"; do
    MATCHING=$(echo "$PR_JSON" | python3 -c "
import sys, json
data = json.load(sys.stdin)
target = sys.argv[1]
for pr in data:
    for f in pr.get('files', []):
        if f['path'] == target:
            print(f'PR #{pr[\"number\"]} ({pr[\"headRefName\"]})')
            break
" "$file" 2>/dev/null || true)
    if [ -n "$MATCHING" ]; then
      echo "  CONFLICT  $file"
      echo "$MATCHING" | while read -r line; do echo "            ↳ $line"; done
      CONFLICTS_FOUND=1
    fi
  done
fi

# 2. Check local claims registry
python3 - "$CLAIMS_FILE" "${FILES_TO_CHECK[@]}" <<'PYEOF'
import sys, json

claims_file = sys.argv[1]
files_to_check = sys.argv[2:]
found = False

with open(claims_file) as f:
    data = json.load(f)

for file in files_to_check:
    c = data.get("claims", {}).get(file)
    if c:
        print(f"  CLAIMED   {file}")
        print(f"            ↳ branch: {c['branch']} (since {c['claimed_at']})")
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
