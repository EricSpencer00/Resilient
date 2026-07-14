#!/usr/bin/env bash
# test-ready-or-bail-closes.sh — regression test for RES-4021:
# ready-or-bail.sh's auto-`Closes #N` heuristic must never close a
# tracker/umbrella issue (e.g. #3933), and must prefer an explicit
# `Closes #N` already in the PR body over deriving one from `Refs #N`.
#
# Sources ready-or-bail.sh (which no-ops its main flow when sourced — see
# the BASH_SOURCE guard near the top of that file) to unit-test
# `compute_close_issue` and `is_tracker_issue` directly, without a real PR
# or any `gh` mutation.
#
# Usage: bash agent-scripts/test-ready-or-bail-closes.sh
# Exits non-zero on failure so CI can gate on it.
#
# shellcheck disable=SC2317,SC2329,SC1091
# ready-or-bail.sh uses a `[[ "${BASH_SOURCE[0]}" != "${0}" ]]; then return`
# guard so it can be safely `source`d here for testing without running its
# full draft-to-ready flow. shellcheck's reachability analysis can't
# resolve that guard as conditional and conservatively marks everything
# below the `source` call as unreachable (SC2317/SC2329 — documented false
# positives, see the SC2317 wiki page: "ignore if invoked indirectly").
# SC1091 is suppressed because shellcheck only follows external sources
# with `-x`; the `shellcheck source=` hint below still documents the path
# for tooling that does pass it.

set -euo pipefail

REPO_ROOT="$(git rev-parse --show-toplevel)"
SCRIPT="$REPO_ROOT/agent-scripts/ready-or-bail.sh"

if [ ! -f "$SCRIPT" ]; then
  echo "FAIL: $SCRIPT does not exist" >&2
  exit 1
fi

# shellcheck source=agent-scripts/ready-or-bail.sh
source "$SCRIPT"

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

fail() { echo "FAIL: $1" >&2; exit 1; }

body_file() {
  local f="$TMP/body.md"
  printf '%s\n' "$1" > "$f"
  printf '%s' "$f"
}

# ---------------------------------------------------------------------------
# Case 1: `Refs #3933 · EPIC` AND an explicit `Closes #<child>` — must NOT
# append Closes #3933; the explicit Closes stays untouched (nothing to add).
# ---------------------------------------------------------------------------
F="$(body_file 'Refs #3933 · F-E4

Implements the thing.

Closes #4021')"
RESULT="$(compute_close_issue "$F")"
[ -z "$RESULT" ] || fail "case1: expected empty, got '$RESULT' (must not touch explicit Closes / must not derive 3933)"
echo "case1 ok: Refs #3933 + explicit Closes #4021 -> no append (tracker skipped, explicit Closes wins)"

# ---------------------------------------------------------------------------
# Case 2: only `Refs #3933 · EPIC` (no child issue) — nothing appended,
# #3933 must never be auto-closed.
# ---------------------------------------------------------------------------
F="$(body_file 'Refs #3933 · F-E4

Just a scaffolding PR, no child ticket yet.')"
RESULT="$(compute_close_issue "$F")"
[ -z "$RESULT" ] || fail "case2: expected empty (tracker), got '$RESULT'"
echo "case2 ok: Refs #3933 alone -> no append"

# ---------------------------------------------------------------------------
# Case 3: only `Refs #<child>` (non-tracker) — Closes #<child> must be
# derived and appended. This is the intended draft->ready convention.
# ---------------------------------------------------------------------------
F="$(body_file 'Refs #55 · some feature

Body text.')"
RESULT="$(compute_close_issue "$F")"
[ "$RESULT" = "55" ] || fail "case3: expected '55', got '$RESULT'"
echo "case3 ok: Refs #55 (non-tracker) -> derives 55"

# ---------------------------------------------------------------------------
# Case 4: body already has `Closes #<child>` (no Refs line at all) —
# unchanged, nothing appended.
# ---------------------------------------------------------------------------
F="$(body_file 'Fixes the bug.

Closes #77')"
RESULT="$(compute_close_issue "$F")"
[ -z "$RESULT" ] || fail "case4: expected empty (explicit Closes already present), got '$RESULT'"
echo "case4 ok: explicit Closes #77 alone -> no append"

# ---------------------------------------------------------------------------
# Case 5: hardcoded denylist covers 3933 WITHOUT needing a `gh` call —
# is_tracker_issue must return true offline (no network / gh mock needed).
# ---------------------------------------------------------------------------
if is_tracker_issue 3933; then
  echo "case5 ok: is_tracker_issue 3933 -> true via hardcoded denylist"
else
  fail "case5: is_tracker_issue 3933 should be true (hardcoded denylist)"
fi

# ---------------------------------------------------------------------------
# Case 6: dynamic tracker detection via injected predicate — simulates an
# issue that isn't on the hardcoded denylist but is labeled tracker/epic/
# umbrella. Uses a mock IS_TRACKER_FN so no real `gh` call is made.
# ---------------------------------------------------------------------------
mock_is_tracker_true() { return 0; }
F="$(body_file 'Refs #999 · some other epic

Body text.')"
RESULT="$(compute_close_issue "$F" mock_is_tracker_true)"
[ -z "$RESULT" ] || fail "case6: expected empty (mock says tracker), got '$RESULT'"
echo "case6 ok: injected tracker predicate blocks auto-close of #999"

mock_is_tracker_false() { return 1; }
F="$(body_file 'Refs #999 · some other epic

Body text.')"
RESULT="$(compute_close_issue "$F" mock_is_tracker_false)"
[ "$RESULT" = "999" ] || fail "case6b: expected '999', got '$RESULT'"
echo "case6b ok: injected non-tracker predicate allows deriving 999"

echo "PASS: test-ready-or-bail-closes.sh"
