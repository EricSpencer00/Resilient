#!/usr/bin/env bash
# Exercise the guarded promotion path with a local bare remote.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SYNC="$SCRIPT_DIR/sync-integration.sh"
TEST_ROOT="$(mktemp -d)"
trap 'rm -rf "$TEST_ROOT"' EXIT

init_repo() {
  local name="$1"
  local remote="$TEST_ROOT/$name.git"
  local repo="$TEST_ROOT/$name"
  git -c init.defaultBranch=main init --bare "$remote" >/dev/null 2>&1
  git init --initial-branch=main "$repo" >/dev/null 2>&1
  git -C "$repo" remote add origin "$remote"
  git -C "$repo" config user.name "integration-promotion-test"
  git -C "$repo" config user.email "integration-promotion-test@example.invalid"
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

# A squash-shaped divergent integration history is repaired through the
# candidate ref, then the candidate is promoted by the guarded sync path.
repo="$(init_repo equivalent)"
commit_file "$repo" base.txt base base
git -C "$repo" push origin HEAD:refs/heads/main >/dev/null 2>&1

git -C "$repo" switch -q -c integration
commit_file "$repo" value.txt shared integration-patch
int_sha="$(git -C "$repo" rev-parse HEAD)"
git -C "$repo" push origin "$int_sha:refs/heads/agents/integration" >/dev/null 2>&1

git -C "$repo" switch -q main
git -C "$repo" switch -q -c candidate
commit_file "$repo" value.txt shared candidate-patch
candidate_sha="$(git -C "$repo" rev-parse HEAD)"
git -C "$repo" push origin "$candidate_sha:refs/heads/candidate" >/dev/null 2>&1

if (cd "$repo" && bash "$SYNC" --pr null --no-push --candidate-ref main) > "$TEST_ROOT/invalid-candidate.log" 2>&1; then
  echo "expected an arbitrary candidate ref to be rejected" >&2
  exit 1
fi
grep -q "candidate ref must identify the current feature branch" "$TEST_ROOT/invalid-candidate.log"
echo "PASS candidate ref restriction"

output="$(cd "$repo" && bash "$SYNC" --pr null --integration agents/integration)"
[[ "$(remote_sha "$repo" refs/heads/agents/integration)" == "$candidate_sha" ]]
grep -Eq "promoted candidate|candidate already included" <<< "$output"
echo "PASS guarded candidate promotion"

# A genuinely unique integration commit must remain protected and must make
# the guarded path fail closed without changing the integration ref.
repo="$(init_repo unique)"
commit_file "$repo" base.txt base base
git -C "$repo" push origin HEAD:refs/heads/main >/dev/null 2>&1

git -C "$repo" switch -q -c integration
commit_file "$repo" unique.txt unique integration-only
int_sha="$(git -C "$repo" rev-parse HEAD)"
git -C "$repo" push origin "$int_sha:refs/heads/agents/integration" >/dev/null 2>&1

git -C "$repo" switch -q main
git -C "$repo" switch -q -c candidate
commit_file "$repo" value.txt candidate candidate-patch
candidate_sha="$(git -C "$repo" rev-parse HEAD)"
git -C "$repo" push origin "$candidate_sha:refs/heads/candidate" >/dev/null 2>&1

if (cd "$repo" && bash "$SYNC" --pr null --integration agents/integration) > "$TEST_ROOT/unique.log" 2>&1; then
  echo "expected unique integration work to fail closed" >&2
  exit 1
fi
[[ "$(remote_sha "$repo" refs/heads/agents/integration)" == "$int_sha" ]]
grep -q "refusing to mark sync complete" "$TEST_ROOT/unique.log"
echo "PASS unique integration protection"
