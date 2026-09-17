#!/usr/bin/env bash
# Exercise every integration-ref state against a local bare remote.

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
  git -C "$repo" config user.name "integration-test"
  git -C "$repo" config user.email "integration-test@example.invalid"
  printf '%s\n' "$repo"
}

commit_value() {
  local repo="$1"
  local value="$2"
  local message="${3:-$value}"
  printf '%s\n' "$value" > "$repo/value.txt"
  git -C "$repo" add value.txt
  git -C "$repo" commit -m "$message" >/dev/null
}

remote_sha() {
  local repo="$1"
  local ref="$2"
  git -C "$repo" ls-remote origin "$ref" | awk 'NR == 1 { print $1 }'
}

fetch_refs() {
  local repo="$1"
  git -C "$repo" fetch origin main agents/integration >/dev/null 2>&1
}

run_reconcile() {
  local repo="$1"
  (cd "$repo" && bash "$RECONCILE" --remote origin)
}

# A missing ref must be created at the current main tip.
repo="$(init_repo missing)"
commit_value "$repo" main
git -C "$repo" push origin HEAD:refs/heads/main >/dev/null 2>&1
git -C "$repo" fetch origin main >/dev/null 2>&1
main_sha="$(git -C "$repo" rev-parse origin/main)"
run_reconcile "$repo"
[[ "$(remote_sha "$repo" refs/heads/agents/integration)" == "$main_sha" ]]
echo "PASS missing"

# A normal lagging ref must still use the fast-forward path.
repo="$(init_repo behind)"
commit_value "$repo" base
git -C "$repo" push origin HEAD:refs/heads/main >/dev/null
base_sha="$(git -C "$repo" rev-parse HEAD)"
commit_value "$repo" main
git -C "$repo" push origin HEAD:refs/heads/main >/dev/null 2>&1
git -C "$repo" push origin "$base_sha:refs/heads/agents/integration" >/dev/null 2>&1
fetch_refs "$repo"
main_sha="$(git -C "$repo" rev-parse origin/main)"
run_reconcile "$repo"
[[ "$(remote_sha "$repo" refs/heads/agents/integration)" == "$main_sha" ]]
echo "PASS behind"

# A ref ahead of main represents work still in flight and must be preserved.
repo="$(init_repo ahead)"
commit_value "$repo" main
git -C "$repo" push origin HEAD:refs/heads/main >/dev/null 2>&1
commit_value "$repo" integration
int_sha="$(git -C "$repo" rev-parse HEAD)"
git -C "$repo" push origin "$int_sha:refs/heads/agents/integration" >/dev/null 2>&1
fetch_refs "$repo"
run_reconcile "$repo"
[[ "$(remote_sha "$repo" refs/heads/agents/integration)" == "$int_sha" ]]
echo "PASS ahead"

# Different commit identities with the same patch are safe after squash merge.
repo="$(init_repo equivalent)"
commit_value "$repo" base
git -C "$repo" push origin HEAD:refs/heads/main >/dev/null
base_sha="$(git -C "$repo" rev-parse HEAD)"
git -C "$repo" switch -q -c integration
commit_value "$repo" shared-change integration-commit
int_sha="$(git -C "$repo" rev-parse HEAD)"
git -C "$repo" push origin "$int_sha:refs/heads/agents/integration" >/dev/null 2>&1
git -C "$repo" switch -q main
git -C "$repo" reset --hard -q "$base_sha"
commit_value "$repo" shared-change main-commit
main_sha="$(git -C "$repo" rev-parse HEAD)"
git -C "$repo" push origin "$main_sha:refs/heads/main" >/dev/null 2>&1
fetch_refs "$repo"
run_reconcile "$repo"
[[ "$(remote_sha "$repo" refs/heads/agents/integration)" == "$main_sha" ]]
echo "PASS patch-equivalent divergence"

# A genuinely unique divergent commit must never be overwritten.
repo="$(init_repo unique)"
commit_value "$repo" base
git -C "$repo" push origin HEAD:refs/heads/main >/dev/null 2>&1
base_sha="$(git -C "$repo" rev-parse HEAD)"
git -C "$repo" switch -q -c integration
commit_value "$repo" unique-integration
int_sha="$(git -C "$repo" rev-parse HEAD)"
git -C "$repo" push origin "$int_sha:refs/heads/agents/integration" >/dev/null 2>&1
git -C "$repo" switch -q main
git -C "$repo" reset --hard -q "$base_sha"
commit_value "$repo" unique-main
git -C "$repo" push origin HEAD:refs/heads/main >/dev/null 2>&1
fetch_refs "$repo"
output="$(run_reconcile "$repo")"
[[ "$(remote_sha "$repo" refs/heads/agents/integration)" == "$int_sha" ]]
grep -q "unique diverging work" <<< "$output"
echo "PASS unique divergence"
