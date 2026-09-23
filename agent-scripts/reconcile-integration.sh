#!/usr/bin/env bash
# Reconcile the shared integration ref with main after the caller has fetched
# both refs. Squash merges can make an already-landed integration commit look
# divergent by object identity, so use patch equivalence before repairing it.

set -euo pipefail

REMOTE="origin"
MAIN_REF=""
INTEGRATION_BRANCH="agents/integration"
PUSH=1

while [[ $# -gt 0 ]]; do
  case "$1" in
    --remote) REMOTE="$2"; shift 2 ;;
    --main-ref) MAIN_REF="$2"; shift 2 ;;
    --integration) INTEGRATION_BRANCH="$2"; shift 2 ;;
    --no-push) PUSH=0; shift ;;
    *) echo "unknown flag: $1" >&2; exit 2 ;;
  esac
done

if [[ -z "$MAIN_REF" ]]; then
  MAIN_REF="$REMOTE/main"
fi
INTEGRATION_REF="$REMOTE/$INTEGRATION_BRANCH"

main_sha="$(git rev-parse --verify "$MAIN_REF^{commit}")"

if ! int_sha="$(git rev-parse --verify "$INTEGRATION_REF^{commit}" 2>/dev/null)"; then
  echo "agents/integration does not exist — creating from main"
  if (( PUSH )); then
    git push "$REMOTE" "$main_sha:refs/heads/$INTEGRATION_BRANCH"
  else
    echo "(--no-push) would create $INTEGRATION_BRANCH at $main_sha"
  fi
  exit 0
fi

if [[ "$main_sha" == "$int_sha" ]]; then
  echo "agents/integration already at main ($main_sha)"
  exit 0
fi

if git merge-base --is-ancestor "$int_sha" "$main_sha"; then
  echo "agents/integration is behind main — fast-forwarding"
  if (( PUSH )); then
    git push "$REMOTE" "$main_sha:refs/heads/$INTEGRATION_BRANCH"
  else
    echo "(--no-push) would fast-forward $INTEGRATION_BRANCH to $main_sha"
  fi
  exit 0
fi

if git merge-base --is-ancestor "$main_sha" "$int_sha"; then
  echo "agents/integration is ahead of main (expected: in-flight work)"
  exit 0
fi

main_tree="$(git rev-parse --verify "$main_sha^{tree}")"
int_tree="$(git rev-parse --verify "$int_sha^{tree}")"
if [[ "$main_tree" == "$int_tree" ]]; then
  echo "agents/integration diverged, but its tree is identical to main (squash-equivalent)"
  if (( PUSH )); then
    # The explicit lease prevents a concurrent agent from being overwritten.
    git push \
      "--force-with-lease=refs/heads/$INTEGRATION_BRANCH:$int_sha" \
      "$REMOTE" "$main_sha:refs/heads/$INTEGRATION_BRANCH"
  else
    echo "(--no-push) would reconcile $INTEGRATION_BRANCH to $main_sha"
  fi
  exit 0
fi

cherry_report="$(git cherry "$main_sha" "$int_sha")"
non_equivalent="$(awk '$1 != "-" { print }' <<< "$cherry_report")"

if [[ -n "$non_equivalent" ]]; then
  echo "::warning::agents/integration has unique diverging work — leaving it untouched"
  echo "  main_sha=$main_sha"
  echo "  int_sha=$int_sha"
  while read -r marker commit subject; do
    [[ "$marker" == "+" ]] || continue
    echo "  unique_commit=$commit ${subject:-}"
  done <<< "$non_equivalent"
  exit 0
fi

echo "agents/integration diverged, but all integration-only commits are patch-equivalent to main"
echo "  main_sha=$main_sha"
echo "  int_sha=$int_sha"
if (( PUSH )); then
  # The explicit lease prevents a concurrent agent from being overwritten.
  git push \
    "--force-with-lease=refs/heads/$INTEGRATION_BRANCH:$int_sha" \
    "$REMOTE" "$main_sha:refs/heads/$INTEGRATION_BRANCH"
else
  echo "(--no-push) would reconcile $INTEGRATION_BRANCH to $main_sha"
fi
