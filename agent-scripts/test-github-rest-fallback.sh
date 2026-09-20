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
    echo "GraphQL: API rate limit already exceeded" >&2
    exit 1
    ;;
  api)
    printf '%s\n' "$*" >> "${MOCK_GH_API_LOG:?}"
    if [[ "${MOCK_GH_API_FAIL:-0}" == 1 ]]; then
      echo "REST failure" >&2
      exit 1
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

if ! github_mark_pr_ready 123 >"$TMP/ready.out" 2>&1; then
  fail "GraphQL ready failure should fall back to REST"
fi
if ! grep -q 'pulls/123' "$MOCK_GH_API_LOG"; then
  fail "REST ready fallback did not PATCH the pull request"
fi
echo "case1 ok: ready transition falls back to REST"

: > "$MOCK_GH_API_LOG"
if ! github_add_pr_label 123 agent-vetted 0E8A16 "guardrail passed" >"$TMP/label.out" 2>&1; then
  fail "GraphQL label failure should fall back to REST"
fi
if ! grep -q 'issues/123/labels' "$MOCK_GH_API_LOG"; then
  fail "REST label fallback did not update the pull request"
fi
echo "case2 ok: label mutation falls back to REST"

: > "$MOCK_GH_API_LOG"
if ! github_comment_pr 123 "handoff body" >"$TMP/comment.out" 2>&1; then
  fail "GraphQL comment failure should fall back to REST"
fi
if ! grep -q 'issues/123/comments' "$MOCK_GH_API_LOG"; then
  fail "REST comment fallback did not create the issue comment"
fi
echo "case3 ok: comment mutation falls back to REST"

export MOCK_GH_API_FAIL=1
if github_mark_pr_ready 123 >"$TMP/rest-failure.out" 2>&1; then
  fail "REST fallback failure must remain fatal"
fi
echo "case4 ok: REST fallback errors remain fatal"

PATH="$OLD_PATH"
echo "PASS: test-github-rest-fallback.sh"
