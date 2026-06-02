#!/usr/bin/env bash
# pick-ticket.sh — select the next agent-ready ticket safe to work on.
#
# Filters:
#   - label `agent-ready`
#   - state open
#   - no open PR whose body or branch references `#<number>`
#   - not assigned (or only the agent bots)
#
# Ordering:
#   - explicit Priority field first (P0 → P3)
#   - then Roadmap goal (lower `G#` first, Infra/tooling last)
#   - then oldest created issue
#
# Output:
#   Prints `<number>\t<title>` for the first eligible ticket, or
#   exits 1 if none.
#
# Usage:
#   agent-scripts/pick-ticket.sh
#   agent-scripts/pick-ticket.sh --json         # full JSON for the ticket
#   agent-scripts/pick-ticket.sh --exclude 123  # skip specific issue

set -euo pipefail

EXCLUDES=()
WANT_JSON=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --json) WANT_JSON=1; shift ;;
    --exclude) EXCLUDES+=("$2"); shift 2 ;;
    *) echo "Unknown flag: $1" >&2; exit 2 ;;
  esac
done

EXCLUDE_CSV="$(IFS=,; echo "${EXCLUDES[*]:-}")"

ISSUES_JSON="$(gh issue list \
  --state open \
  --label agent-ready \
  --limit 100 \
  --json number,title,body,assignees,labels,createdAt 2>/dev/null || echo '[]')"

PRS_JSON="$(gh pr list --state open --limit 100 --json number,title,body,headRefName 2>/dev/null || echo '[]')"

EXCLUDE_CSV="$EXCLUDE_CSV" WANT_JSON="$WANT_JSON" \
ISSUES_JSON="$ISSUES_JSON" PRS_JSON="$PRS_JSON" \
python3 <<'PYEOF'
import json, os, re, sys

issues = json.loads(os.environ.get("ISSUES_JSON") or "[]")
prs = json.loads(os.environ.get("PRS_JSON") or "[]")

excluded = {int(x) for x in os.environ.get("EXCLUDE_CSV", "").split(",") if x.strip().isdigit()}

pr_refs = set()
# Issue-by-number references.
num_re = re.compile(r"(?:closes|fixes|resolves)\s+#(\d+)", re.IGNORECASE)
# RES-NNN references (ticket codename) — resolved via issue titles.
res_re = re.compile(r"\bRES-(\d+)\b", re.IGNORECASE)
# Branch name suffix: any `res-NNN` after optional namespace.
branch_re = re.compile(r"(?:^|/)res-(\d+)", re.IGNORECASE)

res_to_issue = {}
for issue in issues:
    m = res_re.search(issue.get("title") or "")
    if m:
        res_to_issue[int(m.group(1))] = issue["number"]

for pr in prs:
    haystack = " ".join([pr.get("title") or "", pr.get("body") or "", pr.get("headRefName") or ""])
    for m in num_re.finditer(haystack):
        pr_refs.add(int(m.group(1)))
    for m in res_re.finditer(haystack):
        res_num = int(m.group(1))
        if res_num in res_to_issue:
            pr_refs.add(res_to_issue[res_num])
    for m in branch_re.finditer(pr.get("headRefName") or ""):
        res_num = int(m.group(1))
        if res_num in res_to_issue:
            pr_refs.add(res_to_issue[res_num])

heading_re = re.compile(r"^#{1,6}\s+(.+?)\s*$")


def normalize_heading(text):
    return re.sub(r"\s+", " ", text.strip().lower().replace("-", " "))


def sections(body):
    data = {}
    current = None
    buf = []
    for raw in (body or "").splitlines():
        line = raw.strip()
        match = heading_re.match(line)
        if match:
            if current is not None:
                data[current] = "\n".join(buf).strip()
            current = normalize_heading(match.group(1))
            buf = []
            continue
        if current is not None:
            buf.append(raw)
    if current is not None:
        data[current] = "\n".join(buf).strip()
    return data


def section_text(issue, *names):
    sec = sections(issue.get("body") or "")
    return " ".join(sec.get(name, "") for name in names).strip()


def priority_rank(issue):
    text = section_text(issue, "priority")
    match = re.search(r"\bP([0-3])\b", text, re.IGNORECASE)
    if match:
        return int(match.group(1))
    return 2


def roadmap_rank(issue):
    text = section_text(issue, "roadmap goal")
    match = re.search(r"\bG(\d{1,2})\b", text, re.IGNORECASE)
    if match:
        return int(match.group(1))
    lowered = text.lower()
    if "infra" in lowered or "tooling" in lowered:
        return 1000
    return 999


def sort_key(issue):
    return (
        priority_rank(issue),
        roadmap_rank(issue),
        issue.get("createdAt") or "9999-12-31T23:59:59Z",
        issue["number"],
    )


allowed_bots = {"Copilot", "Claude", "claude", "copilot-swe-agent", "anthropic-code-agent"}

eligible = []
for issue in issues:
    n = issue["number"]
    if n in excluded or n in pr_refs:
        continue
    assignees = issue.get("assignees") or []
    non_bot = [a for a in assignees if a.get("login", "") not in allowed_bots]
    if non_bot:
        continue
    eligible.append(issue)

if not eligible:
    sys.exit(1)

eligible.sort(key=sort_key)
selected = eligible[0]

if os.environ.get("WANT_JSON") == "1":
    print(json.dumps(selected))
else:
    print(f"{selected['number']}\t{selected['title']}")

sys.exit(0)
PYEOF
