#!/usr/bin/env bash
# agent-handoff.sh — write a durable, resumable status note to an agent PR.
#
# The note is intentionally a PR comment instead of a checked-in artifact:
# it survives model context loss, follows the branch on GitHub, and does not
# create repository churn for every intermediate agent thought.
#
# Usage:
#   agent-scripts/agent-handoff.sh --pr 123 --issue 57 --phase dispatched \
#     --status "draft opened" --summary "Executor has not started yet."

set -euo pipefail

PR=""
ISSUE=""
PHASE="unknown"
STATUS="unknown"
SUMMARY=""
WORKTREE=""
BRANCH=""
FILES=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --pr) PR="$2"; shift 2 ;;
    --issue) ISSUE="$2"; shift 2 ;;
    --phase) PHASE="$2"; shift 2 ;;
    --status) STATUS="$2"; shift 2 ;;
    --summary) SUMMARY="$2"; shift 2 ;;
    --worktree) WORKTREE="$2"; shift 2 ;;
    --branch) BRANCH="$2"; shift 2 ;;
    --files) FILES="$2"; shift 2 ;;
    *) echo "unknown flag: $1" >&2; exit 2 ;;
  esac
done

if [ -z "$PR" ]; then
  BRANCH="${BRANCH:-$(git rev-parse --abbrev-ref HEAD)}"
  PR="$(gh pr list --head "$BRANCH" --state open --json number -q '.[0].number' 2>/dev/null || true)"
fi

if [ -z "$PR" ] || [ "$PR" = "null" ]; then
  echo "Could not infer open PR. Pass --pr N." >&2
  exit 2
fi

if [ -z "$ISSUE" ]; then
  ISSUE="$(gh pr view "$PR" --json body -q '.body' | sed -nE 's/.*[Cc]loses #([0-9]+).*/\1/p' | head -1 || true)"
fi

BRANCH="${BRANCH:-$(gh pr view "$PR" --json headRefName -q .headRefName 2>/dev/null || true)}"
NOW="$(date -u +"%Y-%m-%dT%H:%M:%SZ")"

FILES_MD="- (none recorded)"
if [ -n "$FILES" ]; then
  FILES_MD="$(printf '%s\n' $FILES | sed 's/^/- `/; s/$/`/')"
fi

BODY="$(cat <<EOF
<!-- resilient-agent-handoff -->
## Agent Handoff

- Time: \`${NOW}\`
- Issue: ${ISSUE:+#${ISSUE}}
- PR: #${PR}
- Branch: \`${BRANCH:-unknown}\`
- Worktree: \`${WORKTREE:-unknown}\`
- Phase: \`${PHASE}\`
- Status: ${STATUS}

### Claimed files
${FILES_MD}

### Resume summary
${SUMMARY:-No summary provided.}
EOF
)"

gh pr comment "$PR" --body "$BODY"
