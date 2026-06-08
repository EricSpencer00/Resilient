#!/usr/bin/env bash
# agent-status.sh — one-screen view of in-flight agent work.
#
# Shows:
#   - active worktrees in .claude/worktrees/
#   - open PRs (by author kind: human / copilot / claude)
#   - queue-health counts for ready-ish, stale, and unclaimed PRs
#   - unclaimed open PRs (no `Closes #N` in body)
#   - stale PRs (>72h without an update)
#   - stale claims whose branch no longer has an open PR
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
  --json number,title,headRefName,baseRefName,isDraft,author,updatedAt,body 2>/dev/null || echo '[]')"
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
QUEUE_HEALTH_JSON="$(PRS_JSON="$PRS_JSON" CLAIMS_JSON="$CLAIMS_JSON" python3 <<'PYEOF'
import datetime as dt
import json
import os
import re

prs = json.loads(os.environ.get("PRS_JSON") or "[]")
claims = json.loads(os.environ.get("CLAIMS_JSON") or '{"claims":{}}').get("claims", {})
now = dt.datetime.utcnow().replace(tzinfo=dt.timezone.utc)
ref_re = re.compile(r"(?:closes|fixes|resolves)\s+#(\d+)", re.IGNORECASE)

main_prs = [pr for pr in prs if pr.get("baseRefName") == "main"]
draft_prs = [pr for pr in prs if pr.get("isDraft")]
readyish_prs = [pr for pr in main_prs if not pr.get("isDraft")]
stale_prs = []
unclaimed_prs = []

for pr in prs:
    updated = pr.get("updatedAt") or ""
    try:
        age_h = int((now - dt.datetime.fromisoformat(updated.replace("Z", "+00:00"))).total_seconds() // 3600)
    except Exception:
        age_h = 0
    if age_h >= 72:
        stale_prs.append({"number": pr["number"], "title": pr.get("title") or "", "age_h": age_h})
    if not ref_re.findall(pr.get("body") or ""):
        unclaimed_prs.append(pr["number"])

open_heads = {pr.get("headRefName") for pr in prs if pr.get("headRefName")}
stale_claims = []
for file_path, info in claims.items():
    branch = info.get("branch") or ""
    if branch and branch not in open_heads:
        stale_claims.append({"file": file_path, "branch": branch})

print(json.dumps({
    "open_prs": len(prs),
    "main_prs": len(main_prs),
    "draft_prs": len(draft_prs),
    "readyish_prs": len(readyish_prs),
    "stale_prs": len(stale_prs),
    "unclaimed_prs": len(unclaimed_prs),
    "claims": len(claims),
    "stale_claim_count": len(stale_claims),
    "draft_pr_numbers": [pr["number"] for pr in draft_prs],
    "readyish_pr_numbers": [pr["number"] for pr in readyish_prs],
    "stale_pr_numbers": [item["number"] for item in stale_prs],
    "unclaimed_pr_numbers": unclaimed_prs,
    "stale_claims": stale_claims,
}))
PYEOF
)"
NEXT_TICKET="$(bash "$(dirname "$0")/pick-ticket.sh" 2>/dev/null || true)"

if (( WANT_JSON == 1 )); then
  WORKTREES_JSON="$WORKTREES_JSON" PRS_JSON="$PRS_JSON" CLAIMS_JSON="$CLAIMS_JSON" QUEUE_HEALTH_JSON="$QUEUE_HEALTH_JSON" NEXT_TICKET="$NEXT_TICKET" python3 <<'PYEOF'
import json
import os

print(json.dumps({
    "worktrees": json.loads(os.environ["WORKTREES_JSON"]),
    "pull_requests": json.loads(os.environ["PRS_JSON"]),
    "file_claims": json.loads(os.environ["CLAIMS_JSON"]).get("claims", {}),
    "queue_health": json.loads(os.environ["QUEUE_HEALTH_JSON"]),
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
    base = "" if pr.get("baseRefName") == "main" else f" [base:{pr.get('baseRefName') or '?'}]"
    print(f"  #{pr['number']:>4} [{kind:<7}] {age_h:>3}h{draft}{stale}{claim}{base}  {pr['title'][:70]}")
PYEOF

echo
bold "Queue health"
QUEUE_HEALTH_JSON="$QUEUE_HEALTH_JSON" python3 <<'PYEOF'
import json
import os

q = json.loads(os.environ["QUEUE_HEALTH_JSON"])
print(f"  open PRs: {q['open_prs']} (main {q['main_prs']}, drafts {q['draft_prs']}, ready-ish {q['readyish_prs']}, stale {q['stale_prs']}, unclaimed {q['unclaimed_prs']})")
print(f"  claims: {q['claims']} (stale {q['stale_claim_count']})")
if q["stale_claims"]:
    print("  stale claims:")
    for claim in q["stale_claims"]:
        print(f"    {claim['file']} ({claim['branch']})")
PYEOF

echo
bold "File claims"
CLAIMS_JSON="$CLAIMS_JSON" QUEUE_HEALTH_JSON="$QUEUE_HEALTH_JSON" python3 <<'PYEOF'
import json, os
data = json.loads(os.environ["CLAIMS_JSON"])
claims = data.get("claims", {})
stale_branches = {item["branch"] for item in json.loads(os.environ["QUEUE_HEALTH_JSON"]).get("stale_claims", [])}
if not claims:
    print("  (none)")
else:
    by_branch = {}
    for f, v in claims.items():
        by_branch.setdefault(v["branch"], []).append(f)
    for b, fs in sorted(by_branch.items()):
        suffix = " [STALE]" if b in stale_branches else ""
        print(f"  {b}{suffix}")
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
