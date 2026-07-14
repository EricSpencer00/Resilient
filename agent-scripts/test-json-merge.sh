#!/usr/bin/env bash
# test-json-merge.sh — regression test for RES-3931: auto-resolve-extensions.sh
# must produce VALID JSON when union-merging conflicting file-claims.json
# entries (the old textual concat dropped the comma separator and, when
# rustfmt was absent, silently shipped invalid JSON).
#
# Usage: bash agent-scripts/test-json-merge.sh
# Exits non-zero on failure so CI can gate on it.

set -euo pipefail

REPO_ROOT="$(git rev-parse --show-toplevel)"
SCRIPT="$REPO_ROOT/agent-scripts/auto-resolve-extensions.sh"

if [ ! -x "$SCRIPT" ]; then
  echo "FAIL: $SCRIPT is not executable" >&2
  exit 1
fi

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

fail() { echo "FAIL: $1" >&2; exit 1; }

# ---------------------------------------------------------------------------
# Case 1: conflicting claims — two branches each appended one entry.
# Union-merging these textually would drop the comma between "a.rs" and
# "b.rs" and emit invalid JSON.
# ---------------------------------------------------------------------------
CLAIMS_DIR="$TMP/agent-scripts"
mkdir -p "$CLAIMS_DIR"
CLAIMS="$CLAIMS_DIR/file-claims.json"

cat > "$CLAIMS" <<'JSON'
{
  "claims": {
    "resilient/src/existing.rs": {
      "branch": "base",
      "claimed_at": "2026-01-01T00:00:00Z"
    },
<<<<<<< ours
    "resilient/src/a.rs": {
      "branch": "branch-a",
      "claimed_at": "2026-07-13T10:00:00Z"
    }
=======
    "resilient/src/b.rs": {
      "branch": "branch-b",
      "claimed_at": "2026-07-13T11:00:00Z"
    }
>>>>>>> theirs
  }
}
JSON

"$SCRIPT" "$CLAIMS" >/dev/null

python3 - "$CLAIMS" <<'PY'
import json, sys
doc = json.load(open(sys.argv[1]))  # raises if invalid JSON -> non-zero exit
claims = doc["claims"]
for k in ("resilient/src/existing.rs", "resilient/src/a.rs", "resilient/src/b.rs"):
    assert k in claims, f"missing claim {k}: {list(claims)}"
assert claims["resilient/src/a.rs"]["branch"] == "branch-a"
assert claims["resilient/src/b.rs"]["branch"] == "branch-b"
print("case1 ok: valid JSON, union of both branches' claims")
PY

grep -q '<<<<<<<\|>>>>>>>\|^=======$' "$CLAIMS" && fail "case1: conflict markers survived"

# ---------------------------------------------------------------------------
# Case 2: collision on the same key — later claimed_at wins, output valid.
# ---------------------------------------------------------------------------
cat > "$CLAIMS" <<'JSON'
{
  "claims": {
<<<<<<< ours
    "resilient/src/x.rs": {
      "branch": "older",
      "claimed_at": "2026-07-13T10:00:00Z"
    }
=======
    "resilient/src/x.rs": {
      "branch": "newer",
      "claimed_at": "2026-07-13T12:00:00Z"
    }
>>>>>>> theirs
  }
}
JSON

"$SCRIPT" "$CLAIMS" >/dev/null
python3 - "$CLAIMS" <<'PY'
import json, sys
doc = json.load(open(sys.argv[1]))
assert doc["claims"]["resilient/src/x.rs"]["branch"] == "newer", doc
print("case2 ok: collision resolved to later claimed_at")
PY

# ---------------------------------------------------------------------------
# Case 3: no conflict markers — a clean JSON file is left untouched.
# ---------------------------------------------------------------------------
printf '{\n  "claims": {}\n}\n' > "$CLAIMS"
before="$(cat "$CLAIMS")"
"$SCRIPT" "$CLAIMS" >/dev/null
[ "$before" = "$(cat "$CLAIMS")" ] || fail "case3: clean file was modified"
echo "case3 ok: clean file untouched"

# ---------------------------------------------------------------------------
# Case 4: .rs extension-block path still union-merges both sides.
# ---------------------------------------------------------------------------
RS_DIR="$TMP/resilient/src"
mkdir -p "$RS_DIR"
RS="$RS_DIR/typechecker.rs"
cat > "$RS" <<'RUST'
pub use crate::a::A;
<<<<<<< ours
pub use crate::b::B;
=======
pub use crate::c::C;
>>>>>>> theirs
RUST

"$SCRIPT" "$RS" >/dev/null
grep -q 'pub use crate::b::B;' "$RS" || fail "case4: ours side lost"
grep -q 'pub use crate::c::C;' "$RS" || fail "case4: theirs side lost"
grep -q '<<<<<<<\|>>>>>>>' "$RS" && fail "case4: markers survived"
echo "case4 ok: .rs union-merge keeps both sides"

echo "PASS: test-json-merge.sh"
