#!/usr/bin/env bash
# Exercise sync-integration.sh against a temporary bare remote.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SYNC="$SCRIPT_DIR/sync-integration.sh"
TEST_ROOT="$(mktemp -d)"
trap 'rm -rf "$TEST_ROOT"' EXIT

GH_BIN="$TEST_ROOT/bin"
mkdir -p "$GH_BIN"
REAL_GIT="$(command -v git)"
printf '%s\n' \
  '#!/usr/bin/env bash' \
  'set -euo pipefail' \
  'if [[ "${1:-}" != "pr" || "${2:-}" != "edit" ]]; then' \
  '  echo "unexpected gh invocation" >&2' \
  '  exit 2' \
  'fi' \
  'printf "%s\n" integration-synced > "$GH_LABEL_FILE"' \
  > "$GH_BIN/gh"
chmod +x "$GH_BIN/gh"

{
  printf '%s\n' '#!/usr/bin/env bash'
  printf 'real_git=%q\n' "$REAL_GIT"
  printf '%s\n' \
    'if [[ "${1:-}" == push ]]; then' \
    '  integration_push=0' \
    '  for arg in "$@"; do' \
    '    [[ "$arg" == HEAD:refs/heads/agents/integration ]] && integration_push=1'
  printf '%s\n' \
    '  done' \
    '  if (( integration_push == 1 )); then' \
    '    pushes=0' \
    '    [[ -f "$SYNC_TEST_GIT_COUNTER" ]] && pushes="$(<"$SYNC_TEST_GIT_COUNTER")"' \
    '    pushes=$((pushes + 1))' \
    '    printf "%s\n" "$pushes" > "$SYNC_TEST_GIT_COUNTER"' \
    '    if (( pushes <= SYNC_TEST_GIT_REJECTIONS )); then' \
    '      echo "simulated integration rejection $pushes" >&2' \
    '      exit 1' \
    '    fi' \
    '  fi' \
    'fi' \
    'exec "$real_git" "$@"'
} > "$GH_BIN/git"
chmod +x "$GH_BIN/git"

init_repo() {
  local name="$1"
  local remote="$TEST_ROOT/$name.git"
  local repo="$TEST_ROOT/$name"

  git -c init.defaultBranch=main init --bare "$remote" >/dev/null 2>&1
  git init --initial-branch=main "$repo" >/dev/null 2>&1
  git -C "$repo" remote add origin "$remote"
  git -C "$repo" config user.name "sync-integration-test"
  git -C "$repo" config user.email "sync-integration-test@example.invalid"
  printf '%s\n' base > "$repo/value.txt"
  git -C "$repo" add value.txt
  git -C "$repo" commit -m base >/dev/null
  git -C "$repo" push origin HEAD:refs/heads/main >/dev/null 2>&1
  git -C "$repo" push origin HEAD:refs/heads/agents/integration >/dev/null 2>&1
  git -C "$repo" switch -q -c sync-test
  printf '%s\n' feature > "$repo/value.txt"
  git -C "$repo" commit -am feature >/dev/null
  git -C "$repo" fetch origin main agents/integration >/dev/null 2>&1
  printf '%s\n' "$repo"
}

remote_sha() {
  local repo="$1"
  git -C "$repo" ls-remote origin refs/heads/agents/integration | awk 'NR == 1 { print $1 }'
}

run_sync() {
  local repo="$1"
  local label_file="$2"
  local counter="$3"
  local rejected_pushes="$4"
  (cd "$repo" && PATH="$GH_BIN:$PATH" GH_LABEL_FILE="$label_file" \
    SYNC_TEST_GIT_COUNTER="$counter" SYNC_TEST_GIT_REJECTIONS="$rejected_pushes" \
    bash "$SYNC" --pr 4515 --integration agents/integration)
}

assert_success_case() {
  local name="$1"
  local rejected_pushes="$2"
  local repo
  local label_file="$TEST_ROOT/$name-label"
  local counter="$TEST_ROOT/$name-integration-pushes"
  local output
  local feature_sha

  repo="$(init_repo "$name")"
  feature_sha="$(git -C "$repo" rev-parse HEAD)"
  output="$(run_sync "$repo" "$label_file" "$counter" "$rejected_pushes" 2>&1)"

  [[ "$(remote_sha "$repo")" == "$feature_sha" ]]
  [[ "$(<"$label_file")" == integration-synced ]]
  [[ "$(<"$counter")" == "$((rejected_pushes + 1))" ]]
  grep -q "sync complete" <<< "$output"
  echo "PASS $name"
}

assert_exhausted_case() {
  local name="exhausted"
  local repo
  local label_file="$TEST_ROOT/$name-label"
  local counter="$TEST_ROOT/$name-integration-pushes"
  local state_file="$TEST_ROOT/$name-pr-state"
  local output
  local status
  local base_sha

  repo="$(init_repo "$name")"
  base_sha="$(git -C "$repo" rev-parse origin/agents/integration)"
  printf '%s\n' draft > "$state_file"

  if output="$(run_sync "$repo" "$label_file" "$counter" 3 2>&1)"; then
    status=0
  else
    status=$?
  fi

  [[ "$status" -ne 0 ]]
  [[ "$(remote_sha "$repo")" == "$base_sha" ]]
  [[ ! -e "$label_file" ]]
  [[ "$(<"$state_file")" == draft ]]
  [[ "$(<"$counter")" == 3 ]]
  grep -q "failed after 3 attempts" <<< "$output"
  ! grep -q "stamped PR" <<< "$output"
  echo "PASS $name"
}

assert_success_case success 0
assert_success_case retry-success 1
assert_exhausted_case
echo "PASS test-sync-integration.sh"
