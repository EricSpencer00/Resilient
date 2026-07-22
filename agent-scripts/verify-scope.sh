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

# Rule 1a: agent PRs must contain real work. The initial claim commit and
# file-claims registry changes are orchestration metadata, not an improvement.
#
# RES-3976: claims now live on the dedicated `agent-claims` ref and are
# never committed onto a feature branch, so `agent-scripts/file-claims.json`
# should no longer appear in this diff at all. The exclusion stays as a
# defensive no-op in case an out-of-date `claim-files.sh` (cached locally by
# an agent that hasn't re-synced tooling) still commits it.
SUBSTANTIVE_FILES=()
for f in "${MODIFIED_FILES[@]}" "${ADDED_FILES[@]}" "${DELETED_FILES[@]}"; do
  case "$f" in
    ""|agent-scripts/file-claims.json) ;;
    *) SUBSTANTIVE_FILES+=("$f") ;;
  esac
done
if (( ${#SUBSTANTIVE_FILES[@]} == 0 )); then
  fail "no substantive changes beyond claim metadata — do not ready or merge empty/claim-only PRs"
fi

# Rule 1b: coauthor trailers must be explicitly configured.
COAUTHOR_TRAILERS="$(git log --format=%B "${BASE}..${HEAD}" 2>/dev/null \
  | grep -iE '^Co-Authored-By:' || true)"
if [ -n "$COAUTHOR_TRAILERS" ]; then
  if [ -z "${AGENT_COAUTHOR_NAME:-}" ] || [ -z "${AGENT_COAUTHOR_EMAIL:-}" ]; then
    fail "Co-Authored-By trailer present but AGENT_COAUTHOR_NAME/EMAIL not configured"
  else
    EXPECTED_TRAILER="Co-Authored-By: ${AGENT_COAUTHOR_NAME} <${AGENT_COAUTHOR_EMAIL}>"
    UNEXPECTED_TRAILERS="$(printf '%s\n' "$COAUTHOR_TRAILERS" \
      | grep -Fivx "$EXPECTED_TRAILER" || true)"
    if [ -n "$UNEXPECTED_TRAILERS" ]; then
      fail "Co-Authored-By trailer does not match configured executor identity"
    fi
  fi
fi

# Rule 1c: existing tests can't be weakened.
#
# We allow modifications to test files (mechanical renames across the
# suite are common — e.g. when a public API or env var name changes),
# but block changes that remove `#[test]` annotations, assertion calls
# (`assert!`, `assert_eq!`, `assert_ne!`, `panic!`, `should_panic`),
# or whole test files.
#
# Golden `.expected.txt` sidecars stay hard-blocked: regenerating one
# is an explicit decision that needs maintainer eyes (the verifier or
# interpreter output changed).
for f in "${DELETED_FILES[@]}"; do
  case "$f" in
    resilient/tests/*.rs|resilient-runtime/tests/*.rs|fuzz/fuzz_targets/*)
      fail "deletes existing test file: $f — requires maintainer approval" ;;
    *.expected.txt)
      fail "deletes existing golden sidecar: $f — requires maintainer approval" ;;
  esac
done
for f in "${MODIFIED_FILES[@]}"; do
  case "$f" in
    *.expected.txt)
      fail "modifies existing golden sidecar: $f — requires maintainer approval"
      continue ;;
  esac
  case "$f" in
    resilient/tests/*.rs|resilient-runtime/tests/*.rs|fuzz/fuzz_targets/*)
      # Count assertion/test-fn lines added vs removed in the diff.
      # If more were removed than added, the test got weaker.
      ASSERT_RX='^[+-][[:space:]]*(#\[test\]|#\[should_panic|assert(_eq|_ne)?!|panic!)'
      ADDED_ASSERTS=$(git diff "${BASE}...${HEAD}" -- "$f" 2>/dev/null \
                       | grep -E "$ASSERT_RX" | grep -cE '^\+' || true)
      REMOVED_ASSERTS=$(git diff "${BASE}...${HEAD}" -- "$f" 2>/dev/null \
                       | grep -E "$ASSERT_RX" | grep -cE '^-' || true)
      if (( REMOVED_ASSERTS > ADDED_ASSERTS )); then
        fail "weakens existing test: $f (removed $REMOVED_ASSERTS test/assert lines, added $ADDED_ASSERTS) — requires maintainer approval"
      fi ;;
  esac
done

# Rule 1b: no new `unsafe` blocks. Scan the diff for added `unsafe` lines.
if git diff "${BASE}...${HEAD}" -- '*.rs' 2>/dev/null | grep -E '^\+.*\bunsafe\b' | grep -vE '^\+\+\+' >/dev/null; then
  fail "introduces new \`unsafe\` block — requires explicit maintainer approval (see CLAUDE.md Security rules)"
fi

# Rule 1c: CI workflows can't be gutted or bypassed.
#
# Path / env / version updates are routine (e.g. when a binary is
# renamed). What we block is the actual harm modes: removing jobs,
# adding bypass patterns (`if: false`, `continue-on-error: true`,
# `--no-verify`), widening permissions, or deleting whole workflows.
for f in "${DELETED_FILES[@]}"; do
  case "$f" in
    .github/workflows/*)
      fail "deletes CI workflow: $f — requires maintainer approval" ;;
  esac
done
for f in "${MODIFIED_FILES[@]}" "${ADDED_FILES[@]}"; do
  case "$f" in
    .github/workflows/*)
      WF_DIFF=$(git diff "${BASE}...${HEAD}" -- "$f" 2>/dev/null || true)
      # Bypass patterns added to the workflow.
      if echo "$WF_DIFF" | grep -E '^\+' | grep -vE '^\+\+\+' \
         | grep -qE '(^\+[[:space:]]*if:[[:space:]]*false\b|continue-on-error:[[:space:]]*true|--no-verify\b|permissions:[[:space:]]*write-all)'; then
        fail "introduces a CI bypass / permission elevation in $f — requires maintainer approval"
      fi
      # Job count drops (top-level `jobs.<name>:` lines).
      if [[ -f "$f" ]]; then
        BEFORE_JOBS=$(git show "${BASE}:$f" 2>/dev/null | awk '/^jobs:/{flag=1;next} flag && /^[A-Za-z]/{flag=0} flag && /^  [A-Za-z_][A-Za-z0-9_-]*:[[:space:]]*$/' | wc -l | tr -d ' ')
        AFTER_JOBS=$(awk '/^jobs:/{flag=1;next} flag && /^[A-Za-z]/{flag=0} flag && /^  [A-Za-z_][A-Za-z0-9_-]*:[[:space:]]*$/' "$f" | wc -l | tr -d ' ')
        if [[ -n "$BEFORE_JOBS" && -n "$AFTER_JOBS" && "$AFTER_JOBS" -lt "$BEFORE_JOBS" ]]; then
          fail "removes a job from $f (was $BEFORE_JOBS, now $AFTER_JOBS) — requires maintainer approval"
        fi
      fi ;;
  esac
done

# Rule 1d: bounded blast radius. Tune as the repo grows. Codebase-wide
# mechanical refactors (e.g. renaming a public symbol or a binary) can
# legitimately touch close to 100 files; the rule's job is to flag
# runaway sed / accidental mass changes, not to block legitimate cross-
# tree edits. Override per-PR with `AGENT_MAX_FILES=N`.
TOTAL_TOUCHED=$(( ${#MODIFIED_FILES[@]} + ${#ADDED_FILES[@]} + ${#DELETED_FILES[@]} ))
MAX_FILES="${AGENT_MAX_FILES:-100}"
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
    # RES-3979: name -> {versions} (a set, not a single version). Cargo.lock
    # legitimately carries two simultaneous semver-major lines of the same
    # crate name (e.g. a new optional dependency pulling in `shlex 2.x`
    # build-side while an existing dependency still pins `shlex 1.x`) —
    # that's an *addition*, not a bump of the existing resolution, and
    # shouldn't trip this check. The old name -> single-version dict
    # silently overwrote same-name entries, so it flagged additions like
    # that as a "1.3.0 -> 2.0.1 bump" even though 1.3.0 was still pinned.
    out = {}
    cur = None
    for line in text.splitlines():
        if line.startswith("name = "):
            cur = line.split('"')[1]
        elif line.startswith("version = ") and cur:
            out.setdefault(cur, set()).add(line.split('"')[1])
            cur = None
    return out
old, new = versions(read(base)), versions(read(head))
bad = []
for name, new_vs in new.items():
    old_vs = old.get(name, set())
    removed, added = old_vs - new_vs, new_vs - old_vs
    if not removed or not added:
        # Nothing disappeared (pure addition) or nothing appeared (pure
        # removal) — neither replaces an existing pin, so it's not a bump.
        continue
    for ov in removed:
        om = re.match(r"(\d+)\.(\d+)", ov)
        for nv in added:
            nm = re.match(r"(\d+)\.(\d+)", nv)
            if om and nm and (om.group(1), om.group(2)) != (nm.group(1), nm.group(2)):
                bad.append(f"{name}: {ov} → {nv}")
if bad:
    print("non-patch bumps:", ", ".join(bad))
    sys.exit(1)
PYEOF
fi

# Rule 1f: claim commit stays at the bottom (if present). It is only claim
# metadata; ready-or-bail adds "Closes #N" after substantive guardrails pass.
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
  # The repo has no top-level Cargo.toml, so `cargo fmt` from the
  # repo root cannot find a manifest. Point it at the workspace
  # crate the same way the clippy/test invocations below do.
  run_cargo "cargo fmt --check" cargo fmt --manifest-path resilient/Cargo.toml --all -- --check
fi
if (( SKIP_CLIPPY == 0 )); then
  run_cargo "cargo clippy -D warnings" cargo clippy --manifest-path resilient/Cargo.toml --all-targets -- -D warnings
fi
if (( SKIP_TESTS == 0 )); then
  run_cargo "cargo test" cargo test --manifest-path resilient/Cargo.toml --quiet
fi

# --- 3. Overlap with other open PRs -------------------------------------------
# Overlaps on the append-only extension allowlist (lib.rs, typechecker.rs,
# lexer_logos.rs) are auto-resolved by sync-integration.sh and are NEVER
# blocking. Only non-allowlist overlaps fail the guardrail.
#
# RES-3976: agent-scripts/file-claims.json used to be listed here too — it
# no longer needs an overlap exemption because claims live on the dedicated
# `agent-claims` ref and are never committed onto a feature branch, so this
# path can't appear in any open PR's file list anymore.
EXTENSION_ALLOWLIST=(
  "resilient/src/lib.rs"
  "resilient/src/main.rs"
  "resilient/src/typechecker.rs"
  "resilient/src/lexer_logos.rs"
)

if [ -x "$REPO_ROOT/agent-scripts/check-overlaps.sh" ]; then
  # Only run in orchestrator mode (network available). --pr-files speaks to gh.
  if command -v gh >/dev/null 2>&1; then
    # check-overlaps.sh filters the current PR out of its own results by
    # comparing the argument against each open PR's headRefName. $HEAD
    # defaults to the literal "HEAD", which matches no branch name — so
    # every PR conflicted with itself. Resolve it to the branch name.
    OVERLAP_REF="$HEAD"
    if [ "$OVERLAP_REF" = "HEAD" ]; then
      OVERLAP_REF="$(git rev-parse --abbrev-ref HEAD)"
    fi
    if ! bash "$REPO_ROOT/agent-scripts/check-overlaps.sh" --pr-files "$OVERLAP_REF" >/tmp/agent-overlap.log 2>&1; then
      # Filter out allowlisted files — overlaps there are expected and handled
      # by sync-integration.sh's auto-resolve step. Only fail on others.
      NON_ALLOWLIST_CONFLICTS=0
      while IFS= read -r line; do
        if [[ "$line" == "  CONFLICT"* ]]; then
          conflicted_file="${line#  CONFLICT  }"
          is_allowed=0
          for allowed in "${EXTENSION_ALLOWLIST[@]}"; do
            [ "$conflicted_file" = "$allowed" ] && is_allowed=1 && break
          done
          [ $is_allowed -eq 0 ] && NON_ALLOWLIST_CONFLICTS=$((NON_ALLOWLIST_CONFLICTS + 1))
        fi
      done < /tmp/agent-overlap.log
      if [ $NON_ALLOWLIST_CONFLICTS -gt 0 ]; then
        fail "overlap detected on non-extension files — see /tmp/agent-overlap.log"
      else
        pass "extension-block overlaps only (auto-resolved by sync-integration.sh)"
      fi
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
