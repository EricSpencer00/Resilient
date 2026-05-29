#!/usr/bin/env bash
# test-claim-sweep.sh — smoke test for RES-2670 stale-claim sweep in
# claim-files.sh.
#
# Usage: bash agent-scripts/test-claim-sweep.sh
#
# Sets up an isolated CLAIMS_FILE in a temp dir, seeds it with one
# claim from a real live branch (`main`) and one from a fake dead
# branch, then runs `claim-files.sh`. Verifies the dead claim was
# swept and the live claim retained. Exits non-zero on failure so CI
# can wire this into the agent-guardrails workflow.

set -euo pipefail

REPO_ROOT="$(git rev-parse --show-toplevel)"
SCRIPT="$REPO_ROOT/agent-scripts/claim-files.sh"

if [ ! -x "$SCRIPT" ]; then
  echo "FAIL: $SCRIPT is not executable" >&2
  exit 1
fi

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

# Make the script see a clean claims file with one stale and one live
# entry. We re-point CLAIMS_FILE by faking a repo root via env? Easier:
# move the real file aside, restore on exit.
REAL="$REPO_ROOT/agent-scripts/file-claims.json"
BACKUP="$TMP/file-claims.json.bak"
cp "$REAL" "$BACKUP"
trap 'mv "$BACKUP" "$REAL"; rm -rf "$TMP"' EXIT

cat > "$REAL" <<'JSON'
{
  "claims": {
    "DEAD/path.rs": {
      "branch": "branch-that-does-not-exist-on-remote-RES2670",
      "claimed_at": "2026-01-01T00:00:00Z"
    },
    "LIVE/path.rs": {
      "branch": "main",
      "claimed_at": "2026-01-01T00:00:00Z"
    }
  }
}
JSON

# Run the script claiming a brand-new file; the sweep happens as a
# side-effect of the call.
"$SCRIPT" "test-branch-RES2670" "TEST/path.rs" > "$TMP/out" 2>&1 || {
  cat "$TMP/out" >&2
  echo "FAIL: claim-files.sh exited non-zero" >&2
  exit 1
}

if ! grep -q "Swept 1 stale claim" "$TMP/out"; then
  cat "$TMP/out" >&2
  echo "FAIL: expected sweep of one stale claim" >&2
  exit 1
fi

if ! python3 -c "
import json,sys
d=json.load(open('$REAL'))['claims']
assert 'DEAD/path.rs' not in d, 'stale claim not swept'
assert 'LIVE/path.rs' in d, 'live claim was incorrectly swept'
assert 'TEST/path.rs' in d, 'new claim not added'
"; then
  cat "$REAL" >&2
  echo "FAIL: claims file in unexpected state" >&2
  exit 1
fi

echo "PASS: stale-claim sweep retained live entries and dropped dead ones"
