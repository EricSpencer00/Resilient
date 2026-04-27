#!/usr/bin/env bash
# dispatch-agent.sh — end-to-end dispatch for an agent-ready ticket.
#
# Does, in order:
#   1. Picks the next agent-ready ticket (or uses --issue N).
#   2. Creates a fresh git worktree at .claude/worktrees/res-<N>/ on a
#      new branch `res-<N>-<short-slug>` based on origin/main.
#   3. Extracts expected file ownership from the issue form and refuses
#      overlapping dispatches before the agent starts editing.
#   4. Opens a draft PR against origin/main with `Closes #<N>` so the
#      ticket is visibly claimed.
#   5. Prints the worktree path + branch + PR URL.
#
# Usage:
#   agent-scripts/dispatch-agent.sh
#   agent-scripts/dispatch-agent.sh --issue 167
#   agent-scripts/dispatch-agent.sh --issue 167 --dry-run
#   agent-scripts/dispatch-agent.sh --issue 167 --claim resilient/src/main.rs
#   agent-scripts/dispatch-agent.sh --issue 167 --no-auto-claim
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
AUTO_CLAIM=1
CLAIM_FILES=()

while [[ $# -gt 0 ]]; do
  case "$1" in
    --issue) ISSUE="$2"; shift 2 ;;
    --claim) CLAIM_FILES+=("$2"); shift 2 ;;
    --no-auto-claim) AUTO_CLAIM=0; shift ;;
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
  ISSUE_BODY="$(gh issue view "$ISSUE" --json body -q .body)"
else
  ISSUE_JSON="$(gh issue view "$ISSUE" --json title,body)"
  TITLE="$(printf '%s' "$ISSUE_JSON" | python3 -c 'import json,sys; print(json.load(sys.stdin)["title"])')"
  ISSUE_BODY="$(printf '%s' "$ISSUE_JSON" | python3 -c 'import json,sys; print(json.load(sys.stdin).get("body") or "")')"
fi

if [ "$AUTO_CLAIM" = "1" ]; then
  ISSUE_BODY_FILE="$(mktemp "${TMPDIR:-/tmp}/resilient-issue-body.XXXXXX")"
  ISSUE_CLAIMS_FILE="$(mktemp "${TMPDIR:-/tmp}/resilient-issue-claims.XXXXXX")"
  printf '%s' "$ISSUE_BODY" > "$ISSUE_BODY_FILE"
  python3 - "$PRIMARY_ROOT" "$ISSUE_BODY_FILE" > "$ISSUE_CLAIMS_FILE" <<'PYEOF'
import os
import re
import sys

root = sys.argv[1]
with open(sys.argv[2], encoding="utf-8") as handle:
    body = handle.read().splitlines()
in_section = False
paths = []

heading = re.compile(r"^#{1,6}\s+(.+?)\s*$")
tick = chr(96)
pathish = re.compile(tick + r"([^" + tick + r"]+)" + tick + r"|(?:^|\s)([A-Za-z0-9_.-]+(?:/[A-Za-z0-9_.@+-]+)+)")

for raw in body:
    line = raw.strip()
    h = heading.match(line)
    if h:
        title = h.group(1).lower()
        if title.startswith("files / modules to touch") or title.startswith("files to touch"):
            in_section = True
            continue
        if in_section:
            break
    if not in_section:
        continue
    if not line or line.lower() in {"none", "n/a", "_no response_"}:
        continue
    for match in pathish.finditer(line):
        candidate = next(group for group in match.groups() if group)
        candidate = candidate.strip(".,;:)")
        if candidate.startswith(("http://", "https://")):
            continue
        if candidate.startswith(("-", "*")):
            candidate = candidate[1:]
        if os.path.isabs(candidate):
            continue
        if candidate.startswith((".github/", ".board/", "agent-scripts/", "resilient/", "resilient-runtime/", "fuzz/", "docs/", "vscode-extension/", "benchmarks/")) or os.path.exists(os.path.join(root, candidate)):
            paths.append(candidate)

for path in dict.fromkeys(paths):
    print(path)
PYEOF
  while IFS= read -r f; do
    [ -n "$f" ] && CLAIM_FILES+=("$f")
  done < "$ISSUE_CLAIMS_FILE"
  rm -f "$ISSUE_BODY_FILE"
  rm -f "$ISSUE_CLAIMS_FILE"
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
if [ ${#CLAIM_FILES[@]} -gt 0 ]; then
  echo "Claims  :"
  printf '  %s\n' "${CLAIM_FILES[@]}"
else
  echo "Claims  : (none inferred)"
fi

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

if [ ${#CLAIM_FILES[@]} -gt 0 ]; then
  bash "$SCRIPT_DIR/check-overlaps.sh" "${CLAIM_FILES[@]}"
fi

git -C "$PRIMARY_ROOT" fetch origin main >/dev/null 2>&1
git -C "$PRIMARY_ROOT" worktree add -b "$BRANCH" "$WORKTREE" origin/main

# Open a draft PR so the ticket shows as claimed. File claims, when known,
# live in the claim commit so CI and sibling agents can inspect them.
if [ ${#CLAIM_FILES[@]} -gt 0 ]; then
  (cd "$WORKTREE" && bash "$SCRIPT_DIR/claim-files.sh" "$BRANCH" "${CLAIM_FILES[@]}") >/dev/null
  git -C "$WORKTREE" add agent-scripts/file-claims.json
  git -C "$WORKTREE" commit -m "res-${ISSUE}: claim ticket — ${TITLE}" >/dev/null
else
  git -C "$WORKTREE" commit --allow-empty -m "res-${ISSUE}: claim ticket — ${TITLE}" >/dev/null
fi
git -C "$WORKTREE" push -u origin "$BRANCH" >/dev/null 2>&1

CLAIM_BLOCK="(none inferred from issue)"
if [ ${#CLAIM_FILES[@]} -gt 0 ]; then
  CLAIM_BLOCK="$(printf -- '- %s\n' "${CLAIM_FILES[@]}")"
fi

PR_URL="$(cd "$WORKTREE" && gh pr create --draft --base main --head "$BRANCH" \
  --title "RES-${ISSUE}: ${TITLE#RES-*: }" \
  --body "$(cat <<EOF
Closes #${ISSUE}.

Draft PR auto-opened by \`agent-scripts/dispatch-agent.sh\` to claim the
ticket. The agent will push real work here shortly.

Branch: \`${BRANCH}\`
Worktree: \`${WORKTREE#${PRIMARY_ROOT}/}\`

## Agent claims

${CLAIM_BLOCK}
EOF
)" 2>&1 | tail -1)"

PR_NUMBER="${PR_URL##*/}"
if [[ "$PR_NUMBER" =~ ^[0-9]+$ ]] && [ -x "$SCRIPT_DIR/agent-handoff.sh" ]; then
  "$SCRIPT_DIR/agent-handoff.sh" \
    --pr "$PR_NUMBER" \
    --issue "$ISSUE" \
    --phase dispatched \
    --status "draft PR opened and file claims recorded" \
    --worktree "$WORKTREE" \
    --branch "$BRANCH" \
    --summary "Executor has not started yet." \
    --files "${CLAIM_FILES[*]:-}" >/dev/null || true
fi

echo
echo "Draft PR: ${PR_URL}"
echo
echo "Next steps:"
echo "  cd \"${WORKTREE}\""
echo "  # edit files, commit, push — the draft PR will update automatically"
echo "  # when ready:  agent-scripts/ready-or-bail.sh --pr ${PR_NUMBER:-<pr>}"
