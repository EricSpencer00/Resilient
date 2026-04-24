#!/usr/bin/env bash
# agent-status.sh — one-screen view of in-flight agent work.
#
# Shows:
#   - active worktrees in .claude/worktrees/
#   - open PRs (by author kind: human / copilot / claude)
#   - unclaimed open PRs (no `Closes #N` in body)
#   - stale PRs (>72h without an update)
#
# Usage: agent-scripts/agent-status.sh

set -euo pipefail

GIT_COMMON_DIR="$(git rev-parse --git-common-dir)"
PRIMARY_ROOT="$(cd "$GIT_COMMON_DIR/.." && pwd)"

bold() { printf '\033[1m%s\033[0m\n' "$1"; }
dim()  { printf '\033[2m%s\033[0m\n' "$1"; }

bold "Worktrees"
git -C "$PRIMARY_ROOT" worktree list | awk '{printf "  %-60s %s\n", $1, $3}'

echo
bold "Open PRs"
PRS_JSON="$(gh pr list --state open --limit 100 \
  --json number,title,headRefName,isDraft,author,updatedAt,body 2>/dev/null || echo '[]')"
PRS_JSON="$PRS_JSON" python3 <<'PYEOF'
import json, os, sys, re, datetime as dt

prs = json.loads(os.environ.get("PRS_JSON") or "[]")
if not prs:
    print("  (none)")
    sys.exit(0)

now = dt.datetime.utcnow().replace(tzinfo=dt.timezone.utc)
ref_re = re.compile(r"(?:closes|fixes|resolves)\s+#(\d+)", re.IGNORECASE)

for pr in sorted(prs, key=lambda p: p["updatedAt"]):
    login = pr["author"].get("login") or "?"
    kind = "copilot" if "copilot" in login.lower() else ("claude" if "claude" in login.lower() or "anthropic" in login.lower() else "human")
    age_h = int((now - dt.datetime.fromisoformat(pr["updatedAt"].replace("Z", "+00:00"))).total_seconds() // 3600)
    stale = " [STALE]" if age_h >= 72 else ""
    draft = " [draft]" if pr["isDraft"] else ""
    closes = ref_re.findall(pr.get("body") or "")
    claim = f" closes #{','.join(closes)}" if closes else " [UNCLAIMED]"
    print(f"  #{pr['number']:>4} [{kind:<7}] {age_h:>3}h{draft}{stale}{claim}  {pr['title'][:70]}")
PYEOF

echo
bold "File claims"
CLAIMS_FILE="$PRIMARY_ROOT/agent-scripts/file-claims.json"
if [ -f "$CLAIMS_FILE" ]; then
  python3 - "$CLAIMS_FILE" <<'PYEOF'
import json, sys
with open(sys.argv[1]) as f:
    data = json.load(f)
claims = data.get("claims", {})
if not claims:
    print("  (none)")
else:
    by_branch = {}
    for f, v in claims.items():
        by_branch.setdefault(v["branch"], []).append(f)
    for b, fs in sorted(by_branch.items()):
        print(f"  {b}")
        for f in fs:
            print(f"    {f}")
PYEOF
else
  echo "  (claims file missing — run after #230 merges)"
fi

echo
bold "Next agent-ready ticket"
bash "$(dirname "$0")/pick-ticket.sh" 2>/dev/null || echo "  (none)"
