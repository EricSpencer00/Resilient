#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
reconcile="$script_dir/reconcile-integration.sh"
tmp_root="$(mktemp -d)"
trap 'rm -rf -- "$tmp_root"' EXIT

remote="$tmp_root/origin.git"
repo="$tmp_root/repo"
git init --bare --quiet --initial-branch=main "$remote"
git init --quiet --initial-branch=main "$repo"
cd "$repo"
git config user.name "Integration Test"
git config user.email "integration-test@example.invalid"
git remote add origin "$remote"

printf 'base\n' > state.txt
git add state.txt
git commit --quiet -m base
git push --quiet -u origin main

git checkout --quiet -b agents/integration
printf 'first\n' > state.txt
git commit --quiet -am first
git push --quiet -u origin agents/integration
printf 'first\nsecond\n' > state.txt
git commit --quiet -am second
git push --quiet origin agents/integration

git checkout --quiet main
printf 'first\nsecond\n' > state.txt
git commit --quiet -am squash
# Main has one squash commit with the same final tree as two integration commits.
git push --quiet origin main
git fetch --quiet origin main agents/integration

output="$("$reconcile" --remote origin --main-ref origin/main --integration agents/integration --no-push 2>&1)"
grep -Fq 'tree is identical to main (squash-equivalent)' <<< "$output"
grep -Fq '(--no-push) would reconcile agents/integration to' <<< "$output"

"$reconcile" --remote origin --main-ref origin/main --integration agents/integration
main_sha="$(git ls-remote origin refs/heads/main | awk '{print $1}')"
integration_sha="$(git ls-remote origin refs/heads/agents/integration | awk '{print $1}')"
test "$main_sha" = "$integration_sha"

git fetch --quiet origin main agents/integration
git checkout --quiet -B agents/integration origin/agents/integration
printf 'unique\n' > unique.txt
git add unique.txt
git commit --quiet -m unique
git push --quiet origin agents/integration
git fetch --quiet origin main agents/integration
output="$("$reconcile" --remote origin --main-ref origin/main --integration agents/integration --no-push 2>&1)"
grep -Fq 'agents/integration is ahead of main (expected: in-flight work)' <<< "$output"
main_sha="$(git ls-remote origin refs/heads/main | awk '{print $1}')"
integration_sha="$(git ls-remote origin refs/heads/agents/integration | awk '{print $1}')"
test "$main_sha" != "$integration_sha"

echo 'squash-tree reconciliation regression passed'
