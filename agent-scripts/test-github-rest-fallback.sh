#!/usr/bin/env bash
# Regression tests for REST fallback mutations used by guarded PR scripts.

set -euo pipefail

REPO_ROOT="$(git rev-parse --show-toplevel)"
# shellcheck source=agent-scripts/github-rest-fallback.sh
source "$REPO_ROOT/agent-scripts/github-rest-fallback.sh"

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

fail() { echo "FAIL: $1" >&2; exit 1; }

MOCK_BIN="$TMP/bin"
mkdir -p "$MOCK_BIN"
cat > "$MOCK_BIN/gh" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
case "${1:-}" in
  pr)
    if [[ "${MOCK_GH_PR_MODE:-rate-limit}" == permission ]]; then
      echo "GraphQL: forbidden" >&2
    else
      echo "GraphQL: API rate limit already exceeded" >&2
    fi
    exit 1
    ;;
  api)
    api_stdin=""
    if [[ "$*" == *"--input -"* ]]; then
      api_stdin="$(cat)"
    fi
    printf 'args=%s stdin=%s\n' "$*" "$api_stdin" >> "${MOCK_GH_API_LOG:?}"
    if [[ "${MOCK_GH_API_FAIL:-0}" == 1 ]]; then
      echo "REST failure" >&2
      exit 1
    fi
    if [[ "$*" == *"pulls/123"* && "$*" == *"--jq"* ]]; then
      cat "${MOCK_GH_DRAFT_FILE:?}"
      exit 0
    fi
    if [[ "$*" == *"pulls/123"* && "$*" == *"--method PATCH"* ]]; then
      if [[ "${MOCK_GH_PATCH_IGNORED:-0}" != 1 ]]; then
        printf '%s\n' false > "${MOCK_GH_DRAFT_FILE:?}"
      fi
      exit 0
    fi
    exit 0
    ;;
  *)
    echo "unexpected gh invocation" >&2
    exit 2
    ;;
esac
EOF
chmod +x "$MOCK_BIN/gh"

OLD_PATH="$PATH"
PATH="$MOCK_BIN:$PATH"
export PATH
export MOCK_GH_API_LOG="$TMP/api.log"
export MOCK_GH_DRAFT_FILE="$TMP/draft"
printf '%s\n' true > "$MOCK_GH_DRAFT_FILE"

if ! github_mark_pr_ready 123 >"$TMP/ready.out" 2>&1; then
  fail "GraphQL ready failure should fall back to REST"
fi
if ! grep -q -- '--method PATCH.*pulls/123.*draft=false' "$MOCK_GH_API_LOG"; then
  fail "REST ready fallback did not PATCH the pull request"
fi
echo "case1 ok: ready transition falls back to REST"

: > "$MOCK_GH_API_LOG"
if ! github_add_pr_label 123 agent-vetted 0E8A16 "guardrail passed" >"$TMP/label.out" 2>&1; then
  fail "GraphQL label failure should fall back to REST"
fi
if ! grep -q -- '--method POST.*labels.*name=agent-vetted' "$MOCK_GH_API_LOG" ||
   ! grep -q -- '--method POST.*issues/123/labels.*agent-vetted' "$MOCK_GH_API_LOG"; then
  fail "REST label fallback did not update the pull request"
fi
echo "case2 ok: label mutation falls back to REST"

: > "$MOCK_GH_API_LOG"
if ! github_comment_pr 123 "handoff body" >"$TMP/comment.out" 2>&1; then
  fail "GraphQL comment failure should fall back to REST"
fi
if ! grep -q -- '--method POST.*issues/123/comments.*body=handoff body' "$MOCK_GH_API_LOG"; then
  fail "REST comment fallback did not create the issue comment"
fi
echo "case3 ok: comment mutation falls back to REST"

printf '%s\n' true > "$MOCK_GH_DRAFT_FILE"
export MOCK_GH_PATCH_IGNORED=1
if github_mark_pr_ready 123 >"$TMP/postcondition.out" 2>&1; then
  fail "REST ready fallback must reject an unchanged draft postcondition"
fi
unset MOCK_GH_PATCH_IGNORED
echo "case4 ok: ready fallback verifies the draft postcondition"

printf '%s\n' true > "$MOCK_GH_DRAFT_FILE"
export MOCK_GH_PR_MODE=permission
: > "$MOCK_GH_API_LOG"
if github_mark_pr_ready 123 >"$TMP/graphql-failure.out" 2>&1; then
  fail "non-rate-limit GraphQL failures must remain fatal"
fi
if [[ -s "$MOCK_GH_API_LOG" ]]; then
  fail "non-rate-limit GraphQL failure unexpectedly called REST"
fi
unset MOCK_GH_PR_MODE
echo "case5 ok: non-rate-limit GraphQL failures remain fatal"

export MOCK_GH_API_FAIL=1
printf '%s\n' true > "$MOCK_GH_DRAFT_FILE"
if github_mark_pr_ready 123 >"$TMP/rest-failure.out" 2>&1; then
  fail "REST fallback failure must remain fatal"
fi
if github_add_pr_label 123 agent-vetted 0E8A16 "guardrail passed" >"$TMP/label-failure.out" 2>&1; then
  fail "REST label fallback failure must remain fatal"
fi
if github_comment_pr 123 "handoff body" >"$TMP/comment-failure.out" 2>&1; then
  fail "REST comment fallback failure must remain fatal"
fi
echo "case6 ok: REST fallback errors remain fatal for ready, label, and comment paths"

PATH="$OLD_PATH"
echo "PASS: test-github-rest-fallback.sh"
