#!/usr/bin/env bash
# ready-or-bail.sh — run verify-scope.sh, then either mark the draft PR
# ready for review (green) or post a failure comment and leave it draft.
#
# Usage:
#   agent-scripts/ready-or-bail.sh              # infer PR from current branch
#   agent-scripts/ready-or-bail.sh --pr 232
#   agent-scripts/ready-or-bail.sh --dry-run    # skip gh mutations
#
# This is the ONLY path the orchestrator uses to transition a draft → ready.
# If this script isn't run, the PR stays draft — that's the whole point.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(git rev-parse --show-toplevel)"
cd "$REPO_ROOT"

PR=""
DRY_RUN=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --pr) PR="$2"; shift 2 ;;
    --dry-run) DRY_RUN=1; shift ;;
    *) echo "unknown flag: $1" >&2; exit 2 ;;
  esac
done

if [ -z "$PR" ]; then
  BRANCH="$(git rev-parse --abbrev-ref HEAD)"
  PR="$(gh pr list --head "$BRANCH" --state open --json number -q '.[0].number' 2>/dev/null || true)"
  if [ -z "$PR" ] || [ "$PR" = "null" ]; then
    echo "Could not infer open PR for branch $BRANCH. Pass --pr N." >&2
    exit 2
  fi
fi

REPORT=/tmp/agent-guardrail-report.json

if bash "$SCRIPT_DIR/verify-scope.sh" --report "$REPORT"; then
  echo
  echo "Guardrail green → marking PR #$PR ready for review."
  if (( DRY_RUN == 0 )); then
    gh pr ready "$PR" 2>&1 | tail -2
    gh pr comment "$PR" --body "Guardrail passed ✓ — fmt, clippy, tests, diff-shape, overlap. Marked ready for human review." >/dev/null
  else
    echo "(dry-run)"
  fi
  exit 0
else
  echo
  echo "Guardrail red → leaving PR #$PR as draft and posting failure comment."
  if (( DRY_RUN == 0 )); then
    BODY="$(python3 - "$REPORT" <<'PYEOF'
import json, sys
try:
    r = json.load(open(sys.argv[1]))
except Exception:
    print("Guardrail failed (report missing).")
    sys.exit(0)
lines = ["Guardrail **FAILED** — leaving PR as draft.", "", "Violations:"]
for f in r.get("failures", []):
    lines.append(f"- {f}")
lines += ["", "Fix the items above, push new commits, and re-run `agent-scripts/ready-or-bail.sh`."]
print("\n".join(lines))
PYEOF
)"
    gh pr comment "$PR" --body "$BODY" >/dev/null
  else
    echo "(dry-run) would comment:"
    cat "$REPORT"
  fi
  exit 1
fi
