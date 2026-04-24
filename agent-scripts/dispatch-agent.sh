#!/usr/bin/env bash
# dispatch-agent.sh — end-to-end dispatch for an agent-ready ticket.
#
# Does, in order:
#   1. Picks the next agent-ready ticket (or uses --issue N).
#   2. Creates a fresh git worktree at .claude/worktrees/res-<N>/ on a
#      new branch `res-<N>-<short-slug>` based on origin/main.
#   3. Opens a draft PR against origin/main with `Closes #<N>` so the
#      ticket is visibly claimed.
#   4. Prints the worktree path + branch + PR URL.
#
# Usage:
#   agent-scripts/dispatch-agent.sh
#   agent-scripts/dispatch-agent.sh --issue 167
#   agent-scripts/dispatch-agent.sh --issue 167 --dry-run
#
# Does NOT run the agent itself — that's the caller's job. This
# script only prepares the workspace.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(git rev-parse --show-toplevel)"
# Climb out of any worktree to the primary checkout so worktree add
# writes to the canonical .claude/worktrees directory.
GIT_COMMON_DIR="$(git rev-parse --git-common-dir)"
PRIMARY_ROOT="$(cd "$GIT_COMMON_DIR/.." && pwd)"

ISSUE=""
DRY_RUN=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --issue) ISSUE="$2"; shift 2 ;;
    --dry-run) DRY_RUN=1; shift ;;
    *) echo "Unknown flag: $1" >&2; exit 2 ;;
  esac
done

if [ -z "$ISSUE" ]; then
  LINE="$(bash "$SCRIPT_DIR/pick-ticket.sh" || true)"
  if [ -z "$LINE" ]; then
    echo "No agent-ready tickets available." >&2
    exit 1
  fi
  ISSUE="$(printf '%s' "$LINE" | cut -f1)"
  TITLE="$(printf '%s' "$LINE" | cut -f2-)"
else
  TITLE="$(gh issue view "$ISSUE" --json title -q .title)"
fi

SLUG="$(printf '%s' "$TITLE" \
  | tr '[:upper:]' '[:lower:]' \
  | sed -E 's/^res-[0-9]+: *//; s/[^a-z0-9]+/-/g; s/^-+|-+$//g' \
  | cut -c1-40 | sed -E 's/-+$//')"
BRANCH="res-${ISSUE}-${SLUG}"
WORKTREE="${PRIMARY_ROOT}/.claude/worktrees/res-${ISSUE}"

echo "Ticket  : #${ISSUE} — ${TITLE}"
echo "Branch  : ${BRANCH}"
echo "Worktree: ${WORKTREE}"

if [ "$DRY_RUN" = "1" ]; then
  echo "(dry-run, stopping here)"
  exit 0
fi

if [ -e "$WORKTREE" ]; then
  echo "ERROR: worktree path $WORKTREE already exists" >&2
  exit 1
fi

if git show-ref --verify --quiet "refs/heads/$BRANCH"; then
  echo "ERROR: branch $BRANCH already exists" >&2
  exit 1
fi

git -C "$PRIMARY_ROOT" fetch origin main >/dev/null 2>&1
git -C "$PRIMARY_ROOT" worktree add -b "$BRANCH" "$WORKTREE" origin/main

# Open a draft PR so the ticket shows as claimed. Use an empty commit
# to give gh something to compare against main.
git -C "$WORKTREE" commit --allow-empty -m "res-${ISSUE}: claim ticket — ${TITLE}" >/dev/null
git -C "$WORKTREE" push -u origin "$BRANCH" >/dev/null 2>&1

PR_URL="$(cd "$WORKTREE" && gh pr create --draft --base main --head "$BRANCH" \
  --title "RES-${ISSUE}: ${TITLE#RES-*: }" \
  --body "$(cat <<EOF
Closes #${ISSUE}.

Draft PR auto-opened by \`agent-scripts/dispatch-agent.sh\` to claim the
ticket. The agent will push real work here shortly.

Branch: \`${BRANCH}\`
Worktree: \`${WORKTREE#${PRIMARY_ROOT}/}\`
EOF
)" 2>&1 | tail -1)"

echo
echo "Draft PR: ${PR_URL}"
echo
echo "Next steps:"
echo "  cd \"${WORKTREE}\""
echo "  # edit files, commit, push — the draft PR will update automatically"
echo "  # when ready:  gh pr ready ${PR_URL}"
