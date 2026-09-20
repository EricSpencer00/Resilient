#!/usr/bin/env bash
# Exercise candidate-branch reconciliation against throwaway local remotes.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RECONCILE="$SCRIPT_DIR/reconcile-integration.sh"
TEST_ROOT="$(mktemp -d)"
trap 'rm -rf "$TEST_ROOT"' EXIT

init_repo() {
  local name="$1"
  local remote="$TEST_ROOT/$name.git"
  local repo="$TEST_ROOT/$name"
  git -c init.defaultBranch=main init --bare "$remote" >/dev/null 2>&1
  git init --initial-branch=main "$repo" >/dev/null 2>&1
  git -C "$repo" remote add origin "$remote"
  git -C "$repo" config user.name "integration-candidate-test"
  git -C "$repo" config user.email "integration-candidate-test@example.invalid"
  printf '%s\n' "$repo"
}

commit_file() {
  local repo="$1"
  local file="$2"
  local value="$3"
  local message="$4"
  printf '%s\n' "$value" > "$repo/$file"
  git -C "$repo" add "$file"
  git -C "$repo" commit -m "$message" >/dev/null
}

remote_sha() {
  local repo="$1"
  local ref="$2"
  git -C "$repo" ls-remote origin "$ref" | awk 'NR == 1 { print $1 }'
}

fetch_refs() {
  local repo="$1"
  git -C "$repo" fetch origin main agents/integration candidate >/dev/null 2>&1
}

run_reconcile() {
  local repo="$1"
  (cd "$repo" && bash "$RECONCILE" --remote origin --candidate-ref origin/candidate)
}

# Two integration commits with different identities on the candidate, plus
# candidate-only work, are safe to promote when both stable patches match.
repo="$(init_repo equivalent)"
commit_file "$repo" value.txt base base
git -C "$repo" push origin HEAD:refs/heads/main >/dev/null 2>&1

git -C "$repo" switch -q -c integration
commit_file "$repo" value.txt first integration-first
commit_file "$repo" value.txt second integration-second
int_sha="$(git -C "$repo" rev-parse HEAD)"
git -C "$repo" push origin "$int_sha:refs/heads/agents/integration" >/dev/null 2>&1

git -C "$repo" switch -q main
commit_file "$repo" main-only.txt main main-only
main_sha="$(git -C "$repo" rev-parse HEAD)"
git -C "$repo" push origin "$main_sha:refs/heads/main" >/dev/null 2>&1

git -C "$repo" switch -q -c candidate
commit_file "$repo" value.txt first candidate-first
commit_file "$repo" value.txt second candidate-second
commit_file "$repo" candidate-only.txt candidate candidate-only
candidate_sha="$(git -C "$repo" rev-parse HEAD)"
git -C "$repo" push origin "$candidate_sha:refs/heads/candidate" >/dev/null 2>&1

fetch_refs "$repo"
output="$(run_reconcile "$repo")"
[[ "$(remote_sha "$repo" refs/heads/agents/integration)" == "$candidate_sha" ]]
grep -q "patch-equivalent to candidate" <<< "$output"
echo "PASS candidate multi-commit equivalence"

# A genuinely unique integration commit still refuses promotion even when a
# candidate contains one matching integration patch.
repo="$(init_repo unique)"
commit_file "$repo" value.txt base base
git -C "$repo" push origin HEAD:refs/heads/main >/dev/null 2>&1

git -C "$repo" switch -q -c integration
commit_file "$repo" value.txt shared integration-shared
commit_file "$repo" unique.txt unique integration-unique
int_sha="$(git -C "$repo" rev-parse HEAD)"
git -C "$repo" push origin "$int_sha:refs/heads/agents/integration" >/dev/null 2>&1

git -C "$repo" switch -q main
commit_file "$repo" main-only.txt main main-only
main_sha="$(git -C "$repo" rev-parse HEAD)"
git -C "$repo" push origin "$main_sha:refs/heads/main" >/dev/null 2>&1

git -C "$repo" switch -q -c candidate
commit_file "$repo" value.txt shared candidate-shared
candidate_sha="$(git -C "$repo" rev-parse HEAD)"
git -C "$repo" push origin "$candidate_sha:refs/heads/candidate" >/dev/null 2>&1

fetch_refs "$repo"
output="$(run_reconcile "$repo")"
[[ "$(remote_sha "$repo" refs/heads/agents/integration)" == "$int_sha" ]]
grep -q "unique diverging work" <<< "$output"
echo "PASS candidate unique divergence"

# A candidate that is not based on the current main tip is not trusted to
# replace the integration ref, even if its patch set looks equivalent.
repo="$(init_repo stale-candidate)"
commit_file "$repo" value.txt base base
base_sha="$(git -C "$repo" rev-parse HEAD)"
git -C "$repo" push origin HEAD:refs/heads/main >/dev/null 2>&1

git -C "$repo" switch -q -c integration
commit_file "$repo" value.txt shared integration-shared
int_sha="$(git -C "$repo" rev-parse HEAD)"
git -C "$repo" push origin "$int_sha:refs/heads/agents/integration" >/dev/null 2>&1

git -C "$repo" switch -q main
commit_file "$repo" main-only.txt main main-only
main_sha="$(git -C "$repo" rev-parse HEAD)"
git -C "$repo" push origin "$main_sha:refs/heads/main" >/dev/null 2>&1

git -C "$repo" switch -q -c candidate
git -C "$repo" reset --hard -q "$base_sha"
commit_file "$repo" value.txt shared stale-candidate-shared
candidate_sha="$(git -C "$repo" rev-parse HEAD)"
git -C "$repo" push origin "$candidate_sha:refs/heads/candidate" >/dev/null 2>&1

fetch_refs "$repo"
output="$(run_reconcile "$repo")"
[[ "$(remote_sha "$repo" refs/heads/agents/integration)" == "$int_sha" ]]
grep -q "not based on main" <<< "$output"
echo "PASS stale candidate refusal"
