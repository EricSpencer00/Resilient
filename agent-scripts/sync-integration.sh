#!/usr/bin/env bash
# sync-integration.sh
#
# Rebase the current branch onto `origin/agents/integration` (the shared
# live branch that tracks all in-flight agent work), then push that
# rebased branch back into `agents/integration` via a fast-forward.
#
# Workflow:
#   1. fetch origin.
#   2. rebase current branch onto origin/agents/integration.
#   3. if the rebase hits conflicts only in the append-only extension
#      allowlist, run auto-resolve-extensions.sh and continue.
#      Otherwise abort the rebase and exit nonzero — that needs a human.
#   4. push the rebased branch to origin (feature branch).
#   5. fast-forward push the rebased HEAD into origin/agents/integration
#      so sibling agents see this work immediately.
#   6. stamp the PR with the `integration-synced` label (if --pr given)
#      so agent-auto-merge.yml knows this PR is sync-safe.
#
# Usage:
#   agent-scripts/sync-integration.sh                  # infer PR from branch
#   agent-scripts/sync-integration.sh --pr 300
#   agent-scripts/sync-integration.sh --no-push        # dry run, rebase only
#
# This script is idempotent: running it again after main advances will
# re-sync. The auto-merge workflow requires the most recent push to have
# been a sync-integration run.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(git rev-parse --show-toplevel)"
cd "$REPO_ROOT"

PR=""
PUSH=1
INTEGRATION_REF="agents/integration"

while [[ $# -gt 0 ]]; do
    case "$1" in
        --pr) PR="$2"; shift 2 ;;
        --no-push) PUSH=0; shift ;;
        --integration) INTEGRATION_REF="$2"; shift 2 ;;
        *) echo "unknown flag: $1" >&2; exit 2 ;;
    esac
done

branch="$(git rev-parse --abbrev-ref HEAD)"
if [[ "$branch" == "HEAD" ]]; then
    echo "refuse: detached HEAD — run on a named branch" >&2
    exit 2
fi
if [[ "$branch" == "main" || "$branch" == "$INTEGRATION_REF" ]]; then
    echo "refuse: must run on a feature branch, not $branch" >&2
    exit 2
fi

if [[ -z "$PR" ]]; then
    PR="$(gh pr list --head "$branch" --state open --json number -q '.[0].number' 2>/dev/null || true)"
fi

echo "Syncing $branch against origin/$INTEGRATION_REF"

git fetch origin "$INTEGRATION_REF" main 2>&1 | tail -3

# Ensure clean working tree.
if ! git diff --quiet || ! git diff --cached --quiet; then
    echo "refuse: uncommitted changes — commit or stash first" >&2
    exit 3
fi

# Rebase.
if git rebase "origin/$INTEGRATION_REF"; then
    echo "rebase: clean"
else
    unresolved="$(git diff --name-only --diff-filter=U)"
    if [[ -z "$unresolved" ]]; then
        echo "rebase: stopped without unresolved files — aborting" >&2
        git rebase --abort || true
        exit 4
    fi
    echo "rebase hit conflicts in:"
    printf '  %s\n' $unresolved

    allowlist=(
        "resilient/src/main.rs"
        "resilient/src/typechecker.rs"
        "resilient/src/lexer_logos.rs"
        "agent-scripts/file-claims.json"
    )

    for f in $unresolved; do
        ok=0
        for a in "${allowlist[@]}"; do
            [[ "$f" == "$a" ]] && ok=1 && break
        done
        if [[ $ok -eq 0 ]]; then
            echo "conflict outside allowlist: $f — manual resolution required" >&2
            git rebase --abort || true
            exit 5
        fi
    done

    files_arr=()
    for f in $unresolved; do files_arr+=("$f"); done

    "$SCRIPT_DIR/auto-resolve-extensions.sh" "${files_arr[@]}"
    git add "${files_arr[@]}"
    GIT_EDITOR=true git rebase --continue

    while git status --short | grep -q '^UU\|^AA\|^DD'; do
        unresolved="$(git diff --name-only --diff-filter=U)"
        for f in $unresolved; do
            ok=0
            for a in "${allowlist[@]}"; do
                [[ "$f" == "$a" ]] && ok=1 && break
            done
            if [[ $ok -eq 0 ]]; then
                echo "conflict outside allowlist: $f" >&2
                git rebase --abort || true
                exit 5
            fi
        done
        files_arr=()
        for f in $unresolved; do files_arr+=("$f"); done
        "$SCRIPT_DIR/auto-resolve-extensions.sh" "${files_arr[@]}"
        git add "${files_arr[@]}"
        GIT_EDITOR=true git rebase --continue
    done
fi

echo "rebase complete — HEAD = $(git rev-parse --short HEAD)"

if (( PUSH == 0 )); then
    echo "(--no-push) stopping before push"
    exit 0
fi

# Push the feature branch (force-with-lease is the only safe force here).
# Agents MUST have the lease — if someone else pushed to their branch,
# this refuses.
git push --force-with-lease origin "$branch"

# Fast-forward integration. If someone else moved integration forward
# since our fetch, retry once.
for attempt in 1 2 3; do
    if git push origin "HEAD:refs/heads/$INTEGRATION_REF"; then
        echo "integration: fast-forwarded to $(git rev-parse --short HEAD)"
        break
    fi
    echo "integration push failed (attempt $attempt), refetching and retrying..."
    git fetch origin "$INTEGRATION_REF" 2>&1 | tail -1
    git rebase "origin/$INTEGRATION_REF" || {
        echo "integration: concurrent conflicting change landed — re-run sync-integration" >&2
        exit 6
    }
    git push --force-with-lease origin "$branch"
done

# Stamp the PR if known.
if [[ -n "$PR" && "$PR" != "null" ]]; then
    gh pr edit "$PR" --add-label "integration-synced" >/dev/null 2>&1 || {
        # Label may not exist yet; create it.
        gh label create "integration-synced" --color "0E8A16" \
            --description "PR has been synced with agents/integration via sync-integration.sh" \
            2>/dev/null || true
        gh pr edit "$PR" --add-label "integration-synced" >/dev/null 2>&1 || true
    }
    echo "stamped PR #$PR with integration-synced"
fi

echo "sync complete"
