#!/usr/bin/env bash
# test-claim-sweep.sh — smoke test for the RES-2670 stale-claim sweep and
# the RES-3976 ref-based claim store (agent-scripts/claims-ref.sh).
#
# Usage: bash agent-scripts/test-claim-sweep.sh
#
# Hermetic: uses a throwaway local bare repo as a stand-in "remote" (via
# AGENT_CLAIMS_REMOTE/AGENT_CLAIMS_REF overrides) so it never touches this
# repo's real `agent-claims` ref or `origin`. Exits non-zero on failure so
# CI can wire this into the agent-guardrails workflow.
#
# Covers:
#   1. Stale-claim sweep: a claim from a branch that no longer exists on
#      the remote is dropped; a claim from a live branch is retained.
#   2. Claim conflicts: claiming a file already owned by another (live)
#      branch fails without mutating the ref.
#   3. release-claims.sh releases exactly the claims for its branch and
#      leaves others untouched.
#   4. RES-3976 acceptance: claiming files never modifies the calling
#      repo's working tree or index — proof that two concurrent claims
#      cannot produce a feature-branch diff on file-claims.json.
#   5. Concurrent-write safety: two claim-files.sh invocations racing to
#      update the ref both succeed (one retries via compare-and-swap) and
#      neither clobbers the other's claim.

set -euo pipefail

REPO_ROOT="$(git rev-parse --show-toplevel)"
CLAIM_SCRIPT="$REPO_ROOT/agent-scripts/claim-files.sh"
RELEASE_SCRIPT="$REPO_ROOT/agent-scripts/release-claims.sh"

for script in "$CLAIM_SCRIPT" "$RELEASE_SCRIPT"; do
  if [ ! -x "$script" ]; then
    echo "FAIL: $script is not executable" >&2
    exit 1
  fi
done

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

BARE="$TMP/fake-origin.git"
git init --quiet --bare "$BARE"

export AGENT_CLAIMS_REMOTE="test-claim-sweep-origin-$$"
export AGENT_CLAIMS_REF="agent-claims-test-$$"

cleanup_remote() {
  git remote remove "$AGENT_CLAIMS_REMOTE" >/dev/null 2>&1 || true
}
trap 'cleanup_remote; rm -rf "$TMP"' EXIT

git remote add "$AGENT_CLAIMS_REMOTE" "$BARE"

# Seed one live branch (exists on the fake remote) and leave one branch
# name that never exists, so the sweep has something real to distinguish.
# Built as a fresh orphan commit via plumbing (not `HEAD`) so this works
# from a shallow checkout too — a shallow clone's `HEAD` carries history
# the bare repo can reject as "shallow update not allowed".
LIVE_BRANCH_TREE="$(printf '' | git mktree)"
LIVE_BRANCH_SHA="$(git commit-tree "$LIVE_BRANCH_TREE" -m "seed live branch" </dev/null)"
git push --quiet "$AGENT_CLAIMS_REMOTE" "${LIVE_BRANCH_SHA}:refs/heads/live-branch-RES2670"

# Seed the claims ref directly (bypassing claim-files.sh) with one dead
# and one live claim, so this test doesn't depend on claim-files.sh
# already working correctly to set up its own fixture.
SEED_JSON="$TMP/seed.json"
cat > "$SEED_JSON" <<'JSON'
{
  "claims": {
    "DEAD/path.rs": {
      "branch": "branch-that-does-not-exist-on-remote-RES2670",
      "claimed_at": "2026-01-01T00:00:00Z"
    },
    "LIVE/path.rs": {
      "branch": "live-branch-RES2670",
      "claimed_at": "2026-01-01T00:00:00Z"
    }
  }
}
JSON
SEED_BLOB="$(git hash-object -w -- "$SEED_JSON")"
SEED_TREE="$(printf '100644 blob %s\tfile-claims.json\n' "$SEED_BLOB" | git mktree)"
SEED_COMMIT="$(git commit-tree "$SEED_TREE" -m "seed")"
git push --quiet "$AGENT_CLAIMS_REMOTE" "${SEED_COMMIT}:refs/heads/${AGENT_CLAIMS_REF}"

# --- 1 & 2: sweep + conflict -------------------------------------------------

OUT="$TMP/out"
if ! "$CLAIM_SCRIPT" "test-branch-RES2670" "TEST/path.rs" > "$OUT" 2>&1; then
  cat "$OUT" >&2
  echo "FAIL: claim-files.sh exited non-zero on a plain claim" >&2
  exit 1
fi
if ! grep -q "Swept 1 stale claim" "$OUT"; then
  cat "$OUT" >&2
  echo "FAIL: expected sweep of exactly one stale claim" >&2
  exit 1
fi

READ_TMP="$TMP/state1.json"
git fetch --quiet "$AGENT_CLAIMS_REMOTE" "+refs/heads/${AGENT_CLAIMS_REF}:refs/remotes/${AGENT_CLAIMS_REMOTE}/${AGENT_CLAIMS_REF}"
git show "refs/remotes/${AGENT_CLAIMS_REMOTE}/${AGENT_CLAIMS_REF}:file-claims.json" > "$READ_TMP"

python3 -c "
import json
d = json.load(open('$READ_TMP'))['claims']
assert 'DEAD/path.rs' not in d, 'stale claim not swept'
assert 'LIVE/path.rs' in d, 'live claim was incorrectly swept'
assert 'TEST/path.rs' in d, 'new claim not added'
assert d['TEST/path.rs']['branch'] == 'test-branch-RES2670'
"

# Conflict: a different branch tries to claim the same file a live branch
# already holds. Must fail (non-zero) and must NOT mutate the ref.
git fetch --quiet "$AGENT_CLAIMS_REMOTE" "+refs/heads/${AGENT_CLAIMS_REF}:refs/remotes/${AGENT_CLAIMS_REMOTE}/${AGENT_CLAIMS_REF}"
BEFORE_CONFLICT_SHA="$(git rev-parse "refs/remotes/${AGENT_CLAIMS_REMOTE}/${AGENT_CLAIMS_REF}")"

if "$CLAIM_SCRIPT" "some-other-branch" "LIVE/path.rs" > "$TMP/conflict_out" 2>&1; then
  cat "$TMP/conflict_out" >&2
  echo "FAIL: claiming an already-claimed file from another branch should fail" >&2
  exit 1
fi
if ! grep -q "already claimed" "$TMP/conflict_out"; then
  cat "$TMP/conflict_out" >&2
  echo "FAIL: expected an 'already claimed' error message" >&2
  exit 1
fi

git fetch --quiet "$AGENT_CLAIMS_REMOTE" "+refs/heads/${AGENT_CLAIMS_REF}:refs/remotes/${AGENT_CLAIMS_REMOTE}/${AGENT_CLAIMS_REF}"
AFTER_CONFLICT_SHA="$(git rev-parse "refs/remotes/${AGENT_CLAIMS_REMOTE}/${AGENT_CLAIMS_REF}")"
[ "$AFTER_CONFLICT_SHA" = "$BEFORE_CONFLICT_SHA" ] || {
  echo "FAIL: a rejected claim must not move the ref" >&2
  exit 1
}

echo "PASS: stale-claim sweep + conflict detection work against the ref-based store"

# --- 3: release-claims.sh releases exactly its branch's claims -------------

if ! "$RELEASE_SCRIPT" "test-branch-RES2670" "" > "$TMP/release_out" 2>&1; then
  cat "$TMP/release_out" >&2
  echo "FAIL: release-claims.sh exited non-zero" >&2
  exit 1
fi
if ! grep -q "Released 1 claim(s) for test-branch-RES2670" "$TMP/release_out"; then
  cat "$TMP/release_out" >&2
  echo "FAIL: expected release-claims.sh to release exactly 1 claim" >&2
  exit 1
fi

git fetch --quiet "$AGENT_CLAIMS_REMOTE" "+refs/heads/${AGENT_CLAIMS_REF}:refs/remotes/${AGENT_CLAIMS_REMOTE}/${AGENT_CLAIMS_REF}"
READ_TMP2="$TMP/state2.json"
git show "refs/remotes/${AGENT_CLAIMS_REMOTE}/${AGENT_CLAIMS_REF}:file-claims.json" > "$READ_TMP2"
python3 -c "
import json
d = json.load(open('$READ_TMP2'))['claims']
assert 'TEST/path.rs' not in d, 'released claim still present'
assert 'LIVE/path.rs' in d, 'unrelated live claim was incorrectly released'
"

echo "PASS: release-claims.sh releases exactly the calling branch's claims"

# --- 4: RES-3976 acceptance — claiming never touches the working tree ------

BEFORE_STATUS="$(git status --porcelain)"
"$CLAIM_SCRIPT" "test-branch-workingtree" "WORKINGTREE/path.rs" > /dev/null
AFTER_STATUS="$(git status --porcelain)"
if [ "$BEFORE_STATUS" != "$AFTER_STATUS" ]; then
  echo "FAIL: claim-files.sh modified the working tree/index — this is exactly the RES-3976 regression" >&2
  git status --short >&2
  exit 1
fi
if [ -n "$(git diff --name-only -- agent-scripts/file-claims.json 2>/dev/null)" ]; then
  echo "FAIL: agent-scripts/file-claims.json shows a diff after claiming — would appear in a PR" >&2
  exit 1
fi

echo "PASS: claiming files leaves the working tree and index untouched (no PR-diff regression)"

# --- 5: concurrent claims race safely via compare-and-swap ------------------

# Both racing branches must exist as real branches on the fake remote —
# otherwise the RES-2670 sweep (which drops claims from branches the
# remote doesn't know about) would sweep away whichever branch's claim
# isn't the one currently being written, which is a sweep-correctness
# scenario, not a CAS-race scenario. Reuse the empty-tree orphan-commit
# trick from the earlier seed to stay shallow-clone-safe.
RACE_TREE="$(printf '' | git mktree)"
RACE_SHA="$(git commit-tree "$RACE_TREE" -m "seed race branch" </dev/null)"
git push --quiet "$AGENT_CLAIMS_REMOTE" "${RACE_SHA}:refs/heads/test-branch-race-A"
git push --quiet "$AGENT_CLAIMS_REMOTE" "${RACE_SHA}:refs/heads/test-branch-race-B"

"$CLAIM_SCRIPT" test-branch-race-A RACE/a.rs > "$TMP/race_a" 2>&1 &
PID_A=$!
"$CLAIM_SCRIPT" test-branch-race-B RACE/b.rs > "$TMP/race_b" 2>&1 &
PID_B=$!

RC_A=0
RC_B=0
wait "$PID_A" || RC_A=$?
wait "$PID_B" || RC_B=$?

if [ "$RC_A" -ne 0 ] || [ "$RC_B" -ne 0 ]; then
  echo "--- race_a ---" >&2; cat "$TMP/race_a" >&2
  echo "--- race_b ---" >&2; cat "$TMP/race_b" >&2
  echo "FAIL: two concurrent claims on disjoint files should both succeed via CAS retry" >&2
  exit 1
fi

git fetch --quiet "$AGENT_CLAIMS_REMOTE" "+refs/heads/${AGENT_CLAIMS_REF}:refs/remotes/${AGENT_CLAIMS_REMOTE}/${AGENT_CLAIMS_REF}"
READ_TMP3="$TMP/state3.json"
git show "refs/remotes/${AGENT_CLAIMS_REMOTE}/${AGENT_CLAIMS_REF}:file-claims.json" > "$READ_TMP3"
python3 -c "
import json
d = json.load(open('$READ_TMP3'))['claims']
assert 'RACE/a.rs' in d and d['RACE/a.rs']['branch'] == 'test-branch-race-A', 'race claim A lost'
assert 'RACE/b.rs' in d and d['RACE/b.rs']['branch'] == 'test-branch-race-B', 'race claim B lost'
"

echo "PASS: concurrent claims on disjoint files both land (compare-and-swap retry)"

echo "PASS: test-claim-sweep.sh"
