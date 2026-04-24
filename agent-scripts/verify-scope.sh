#!/usr/bin/env bash
# verify-scope.sh — the guardrail that decides if an agent's work on a
# branch is safe to mark "ready for review".
#
# Layered checks, stopping at the first failure:
#   1. Diff-shape rules (no modified tests, no new unsafe, no workflow
#      edits, bounded file count).
#   2. Format / lint / build / test must all pass.
#   3. Overlap check against other open PRs.
#
# Exit codes:
#   0 — all guardrails green.
#   1 — a guardrail failed; failures printed to stdout AND written to
#       $OUT_JSON as a structured report the orchestrator + GH Action
#       can post back to the PR.
#
# Usage:
#   agent-scripts/verify-scope.sh                # current branch vs origin/main
#   agent-scripts/verify-scope.sh --base origin/main --head HEAD
#   agent-scripts/verify-scope.sh --report /tmp/r.json
#   agent-scripts/verify-scope.sh --skip tests   # skip expensive checks (CI-only)
#
# Safe to run anywhere inside the repo (worktree or primary).

set -o pipefail

BASE="origin/main"
HEAD="HEAD"
OUT_JSON=""
SKIP_TESTS=0
SKIP_CLIPPY=0
SKIP_FMT=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --base) BASE="$2"; shift 2 ;;
    --head) HEAD="$2"; shift 2 ;;
    --report) OUT_JSON="$2"; shift 2 ;;
    --skip)
      case "$2" in
        tests)  SKIP_TESTS=1 ;;
        clippy) SKIP_CLIPPY=1 ;;
        fmt)    SKIP_FMT=1 ;;
        *) echo "unknown --skip target: $2" >&2; exit 2 ;;
      esac
      shift 2 ;;
    *) echo "unknown flag: $1" >&2; exit 2 ;;
  esac
done

REPO_ROOT="$(git rev-parse --show-toplevel)"
cd "$REPO_ROOT"

# Make sure the base ref is fresh. Swallow the output — no network logs.
git fetch origin "${BASE#origin/}" >/dev/null 2>&1 || true

FAILURES=()
fail() { FAILURES+=("$1"); echo "FAIL  $1"; }
pass() { echo "PASS  $1"; }

# --- 1. Diff-shape rules ------------------------------------------------------

MAPFILE_DIFF=()
while IFS= read -r line; do MAPFILE_DIFF+=("$line"); done < <(git diff --name-status "${BASE}...${HEAD}" 2>/dev/null || true)

MODIFIED_FILES=()
ADDED_FILES=()
DELETED_FILES=()
for entry in "${MAPFILE_DIFF[@]}"; do
  status="${entry%%$'\t'*}"
  path="${entry#*$'\t'}"
  case "$status" in
    M*|R*) MODIFIED_FILES+=("$path") ;;
    A*)    ADDED_FILES+=("$path") ;;
    D*)    DELETED_FILES+=("$path") ;;
  esac
done

# Rule 1a: no existing test files modified or deleted.
for f in "${MODIFIED_FILES[@]}" "${DELETED_FILES[@]}"; do
  case "$f" in
    resilient/tests/*.rs|resilient-runtime/tests/*.rs|fuzz/fuzz_targets/*)
      fail "modifies or deletes existing test file: $f" ;;
    *.expected.txt)
      fail "modifies or deletes existing golden sidecar: $f" ;;
  esac
done
# Also block modifications to #[cfg(test)] inline tests is too noisy to detect
# statically — we rely on test-pass to catch that instead.

# Rule 1b: no new `unsafe` blocks. Scan the diff for added `unsafe` lines.
if git diff "${BASE}...${HEAD}" -- '*.rs' 2>/dev/null | grep -E '^\+.*\bunsafe\b' | grep -vE '^\+\+\+' >/dev/null; then
  fail "introduces new \`unsafe\` block — requires explicit maintainer approval (see CLAUDE.md Security rules)"
fi

# Rule 1c: no CI workflow edits.
for f in "${MODIFIED_FILES[@]}" "${ADDED_FILES[@]}" "${DELETED_FILES[@]}"; do
  case "$f" in
    .github/workflows/*) fail "touches CI workflow: $f — requires maintainer approval" ;;
  esac
done

# Rule 1d: bounded blast radius. Tune as the repo grows.
TOTAL_TOUCHED=$(( ${#MODIFIED_FILES[@]} + ${#ADDED_FILES[@]} + ${#DELETED_FILES[@]} ))
MAX_FILES="${AGENT_MAX_FILES:-60}"
if (( TOTAL_TOUCHED > MAX_FILES )); then
  fail "touches $TOTAL_TOUCHED files (> $MAX_FILES). Oversized PR — split or ask for approval."
fi

# Rule 1e: no Cargo.lock major/minor bumps (patch is OK).
if git diff --name-only "${BASE}...${HEAD}" 2>/dev/null | grep -q '^Cargo\.lock$'; then
  # Pull the old + new versions and diff them.
  python3 - "$BASE" "$HEAD" <<'PYEOF' || fail "Cargo.lock contains a non-patch dependency bump — requires approval"
import re, subprocess, sys
base, head = sys.argv[1], sys.argv[2]
def read(ref):
    try:
        return subprocess.check_output(["git", "show", f"{ref}:Cargo.lock"], text=True)
    except subprocess.CalledProcessError:
        return ""
def versions(text):
    out = {}
    cur = None
    for line in text.splitlines():
        if line.startswith("name = "):
            cur = line.split('"')[1]
        elif line.startswith("version = ") and cur:
            out[cur] = line.split('"')[1]
            cur = None
    return out
old, new = versions(read(base)), versions(read(head))
bad = []
for name, nv in new.items():
    ov = old.get(name)
    if not ov or ov == nv:
        continue
    om = re.match(r"(\d+)\.(\d+)", ov)
    nm = re.match(r"(\d+)\.(\d+)", nv)
    if om and nm and (om.group(1), om.group(2)) != (nm.group(1), nm.group(2)):
        bad.append(f"{name}: {ov} → {nv}")
if bad:
    print("non-patch bumps:", ", ".join(bad))
    sys.exit(1)
PYEOF
fi

# Rule 1f: claim commit stays at the bottom (if present) — the orchestrator
# needs the empty claim commit so the PR's "Closes #N" is preserved.
FIRST_COMMIT_MSG=$(git log --format=%s "${BASE}..${HEAD}" --reverse 2>/dev/null | head -1)
if [[ "$FIRST_COMMIT_MSG" =~ ^res-[0-9]+:\ claim\ ticket ]]; then
  pass "claim commit preserved at base of branch"
fi

if (( ${#FAILURES[@]} == 0 )); then
  pass "diff-shape rules (${TOTAL_TOUCHED} files touched)"
fi

# --- 2. Build / test / lint ---------------------------------------------------

run_cargo() {
  local label="$1"; shift
  if "$@" >/tmp/agent-guardrail.log 2>&1; then
    pass "$label"
  else
    fail "$label — see /tmp/agent-guardrail.log"
    tail -40 /tmp/agent-guardrail.log | sed 's/^/       /'
  fi
}

if (( SKIP_FMT == 0 )); then
  run_cargo "cargo fmt --check" cargo fmt --all -- --check
fi
if (( SKIP_CLIPPY == 0 )); then
  run_cargo "cargo clippy -D warnings" cargo clippy --manifest-path resilient/Cargo.toml --all-targets -- -D warnings
fi
if (( SKIP_TESTS == 0 )); then
  run_cargo "cargo test" cargo test --manifest-path resilient/Cargo.toml --quiet
fi

# --- 3. Overlap with other open PRs -------------------------------------------

if [ -x "$REPO_ROOT/agent-scripts/check-overlaps.sh" ]; then
  # Only run in orchestrator mode (network available). --pr-files speaks to gh.
  if command -v gh >/dev/null 2>&1; then
    if ! bash "$REPO_ROOT/agent-scripts/check-overlaps.sh" --pr-files "$HEAD" >/tmp/agent-overlap.log 2>&1; then
      fail "overlap detected against another open PR — see /tmp/agent-overlap.log"
      cat /tmp/agent-overlap.log | sed 's/^/       /'
    else
      pass "no file overlap with other open PRs"
    fi
  fi
fi

# --- Report -------------------------------------------------------------------

if [ -n "$OUT_JSON" ]; then
  python3 - "$OUT_JSON" "${#FAILURES[@]}" "${FAILURES[@]:-}" <<'PYEOF'
import json, sys
path, n = sys.argv[1], int(sys.argv[2])
fails = sys.argv[3:3+n]
json.dump({"passed": n == 0, "failures": fails}, open(path, "w"), indent=2)
PYEOF
fi

if (( ${#FAILURES[@]} > 0 )); then
  echo
  echo "Guardrail FAILED with ${#FAILURES[@]} issue(s)."
  exit 1
fi

echo
echo "Guardrail PASSED."
exit 0
