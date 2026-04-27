#!/usr/bin/env bash
# agent-status.sh — one-screen view of in-flight agent work.
#
# Shows:
#   - active worktrees in .claude/worktrees/
#   - open PRs (by author kind: human / copilot / claude)
#   - unclaimed open PRs (no `Closes #N` in body)
#   - stale PRs (>72h without an update)
#
# Usage:
#   agent-scripts/agent-status.sh
#   agent-scripts/agent-status.sh --json

set -euo pipefail

WANT_JSON=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --json) WANT_JSON=1; shift ;;
    *) echo "unknown flag: $1" >&2; exit 2 ;;
  esac
done

GIT_COMMON_DIR="$(git rev-parse --git-common-dir)"
PRIMARY_ROOT="$(cd "$GIT_COMMON_DIR/.." && pwd)"

bold() { printf '\033[1m%s\033[0m\n' "$1"; }
dim()  { printf '\033[2m%s\033[0m\n' "$1"; }

PRS_JSON="$(gh pr list --state open --limit 100 \
  --json number,title,headRefName,isDraft,author,updatedAt,body 2>/dev/null || echo '[]')"
CLAIMS_FILE="$PRIMARY_ROOT/agent-scripts/file-claims.json"
WORKTREES_RAW="$(git -C "$PRIMARY_ROOT" worktree list --porcelain)"
WORKTREES_JSON="$(WORKTREES_RAW="$WORKTREES_RAW" python3 <<'PYEOF'
import json
import os
import sys

items = []
cur = {}
for line in os.environ.get("WORKTREES_RAW", "").splitlines():
    line = line.rstrip("\n")
    if not line:
        if cur:
            items.append(cur)
            cur = {}
        continue
    key, _, value = line.partition(" ")
    if key == "worktree":
        cur["path"] = value
    elif key == "HEAD":
        cur["head"] = value
    elif key == "branch":
        cur["branch"] = value.removeprefix("refs/heads/")
if cur:
    items.append(cur)
print(json.dumps(items))
PYEOF
)"
CLAIMS_JSON="$(if [ -f "$CLAIMS_FILE" ]; then cat "$CLAIMS_FILE"; else printf '{"claims":{}}'; fi)"
NEXT_TICKET="$(bash "$(dirname "$0")/pick-ticket.sh" 2>/dev/null || true)"

if (( WANT_JSON == 1 )); then
  WORKTREES_JSON="$WORKTREES_JSON" PRS_JSON="$PRS_JSON" CLAIMS_JSON="$CLAIMS_JSON" NEXT_TICKET="$NEXT_TICKET" python3 <<'PYEOF'
import json
import os

print(json.dumps({
    "worktrees": json.loads(os.environ["WORKTREES_JSON"]),
    "pull_requests": json.loads(os.environ["PRS_JSON"]),
    "file_claims": json.loads(os.environ["CLAIMS_JSON"]).get("claims", {}),
    "next_ticket": os.environ.get("NEXT_TICKET") or None,
}, indent=2))
PYEOF
  exit 0
fi

bold "Worktrees"
WORKTREES_JSON="$WORKTREES_JSON" python3 <<'PYEOF'
import json
import os
for wt in json.loads(os.environ["WORKTREES_JSON"]):
    print(f"  {wt.get('path',''):<60} {wt.get('branch','')}")
PYEOF

echo
bold "Open PRs"
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
CLAIMS_JSON="$CLAIMS_JSON" python3 <<'PYEOF'
import json, os
data = json.loads(os.environ["CLAIMS_JSON"])
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

echo
bold "Next agent-ready ticket"
if [ -n "$NEXT_TICKET" ]; then
  printf '%s\n' "$NEXT_TICKET"
else
  echo "  (none)"
fi
