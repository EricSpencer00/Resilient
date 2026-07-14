#!/usr/bin/env bash
# claims-ref.sh — shared helpers for reading/writing file-claims.json on the
# dedicated `agent-claims` ref, instead of committing it into feature-branch
# history.
#
# RES-3976: `claim-files.sh` used to commit `agent-scripts/file-claims.json`
# onto the feature branch. That put the file in every PR's diff, so every
# merge to `main` (which also rewrote the file, via
# `release-file-claims.yml`) staled every other open PR's copy — GitHub
# reported `MERGE: DIRTY` on `file-claims.json` even when every required
# check was green, and the RES-3931 union-merge only papers over conflicts
# in-place; it doesn't stop them recurring on every merge.
#
# The fix: claims live ONLY on a dedicated branch (default `agent-claims`)
# that no feature branch ever merges from or rebases onto, and no feature
# branch ever commits to. Callers treat the ref as a tiny remote key-value
# store:
#
#   claims_fetch_base <out-json-file>
#     Fetch the ref and write its current `file-claims.json` content (or
#     `{"claims": {}}` if the ref does not exist yet) to <out-json-file>.
#     Sets CLAIMS_BASE_SHA to the ref's current commit (or "" if absent).
#
#   claims_try_push <json-file> <commit-message>
#     Build a new commit from <json-file> on top of CLAIMS_BASE_SHA and
#     attempt a *non-force* push. Git's push protocol only accepts the
#     update if the remote ref still matches CLAIMS_BASE_SHA (or is absent,
#     for the first-ever write) — this is a compare-and-swap, so a
#     concurrent writer never gets silently clobbered. Returns 1 on
#     rejection; the caller should re-fetch and retry.
#
#   claims_apply_with_retry <edit-fn> <commit-message> [max-attempts]
#     Retry loop: fetch, run <edit-fn> <json-file> to mutate the file in
#     place, try to push, and retry on CAS rejection. <edit-fn> is a bash
#     function name (called as `"$edit_fn" "$json_file"`), so it runs in
#     the same shell as the caller and can share variables/state via a
#     side file if it needs to report a summary.
#
# Because every write re-fetches and re-applies the edit against the
# latest content, two concurrent writers never need textual conflict
# resolution for this ref — the RES-3931 JSON-aware merge in
# `auto-resolve-extensions.sh` is preserved unmodified for the cases that
# still need it (genuine git-level conflicts on the append-only `.rs`
# extension blocks, and any future manual/rebase-based handling of this
# ref).
#
# Overridable for tests:
#   AGENT_CLAIMS_REMOTE  — remote name (default: origin)
#   AGENT_CLAIMS_REF     — branch name on that remote (default: agent-claims)

set -euo pipefail

claims_remote_name() {
  printf '%s' "${AGENT_CLAIMS_REMOTE:-origin}"
}

claims_ref_name() {
  printf '%s' "${AGENT_CLAIMS_REF:-agent-claims}"
}

# Populates CLAIMS_BASE_SHA (exported var) and writes current content to $1.
claims_fetch_base() {
  local out_file="$1"
  local remote ref
  remote="$(claims_remote_name)"
  ref="$(claims_ref_name)"

  git fetch -q "$remote" "+refs/heads/${ref}:refs/remotes/${remote}/${ref}" >/dev/null 2>&1 || true

  if git rev-parse -q --verify "refs/remotes/${remote}/${ref}" >/dev/null 2>&1; then
    CLAIMS_BASE_SHA="$(git rev-parse "refs/remotes/${remote}/${ref}")"
    if ! git show "${CLAIMS_BASE_SHA}:file-claims.json" > "$out_file" 2>/dev/null; then
      printf '{"claims": {}}' > "$out_file"
    fi
  else
    CLAIMS_BASE_SHA=""
    printf '{"claims": {}}' > "$out_file"
  fi
}

# Attempts a single CAS push of $1 (a file containing the new
# file-claims.json content) as a new commit on top of CLAIMS_BASE_SHA, with
# commit message $2. On success, updates CLAIMS_BASE_SHA to the new commit
# and returns 0. On rejection (someone else moved the ref first) returns 1
# — the caller should re-run claims_fetch_base and retry.
claims_try_push() {
  local json_file="$1" message="$2"
  local remote ref blob tree commit

  remote="$(claims_remote_name)"
  ref="$(claims_ref_name)"

  blob="$(git hash-object -w -- "$json_file")"
  tree="$(printf '100644 blob %s\tfile-claims.json\n' "$blob" | git mktree)"

  if [ -n "${CLAIMS_BASE_SHA:-}" ]; then
    commit="$(git commit-tree "$tree" -p "$CLAIMS_BASE_SHA" -m "$message")"
  else
    commit="$(git commit-tree "$tree" -m "$message")"
  fi

  if git push "$remote" "${commit}:refs/heads/${ref}" >/dev/null 2>&1; then
    CLAIMS_BASE_SHA="$commit"
    return 0
  fi
  return 1
}

# claims_apply_with_retry <edit-fn> <commit-message> [max-attempts]
#
# <edit-fn> is called as `"$edit_fn" "$json_file"` and must mutate
# $json_file in place (it starts out holding the latest fetched content).
# It may write a human-readable summary to "$json_file.summary" — that
# file's contents (if any) are printed once, after the push that actually
# lands, so retried attempts don't print duplicate/stale summaries.
#
# A non-retryable failure (e.g. "file already claimed by another branch")
# should be signalled by <edit-fn> exiting non-zero; claims_apply_with_retry
# propagates that exit code immediately without retrying.
claims_apply_with_retry() {
  local edit_fn="$1" message="$2" max_attempts="${3:-8}"
  local tmp attempt rc
  tmp="$(mktemp)"
  # shellcheck disable=SC2064 # intentional early expansion of $tmp
  trap "rm -f '$tmp' '$tmp.summary'" RETURN

  for (( attempt=1; attempt<=max_attempts; attempt++ )); do
    claims_fetch_base "$tmp"
    rm -f "$tmp.summary"

    if "$edit_fn" "$tmp"; then
      if claims_try_push "$tmp" "$message"; then
        if [ -f "$tmp.summary" ]; then
          cat "$tmp.summary"
        fi
        return 0
      fi
      sleep "0.$(( (RANDOM % 5) + 1 ))"
    else
      # Non-retryable failure (e.g. a real claim conflict) — propagate
      # edit_fn's exit code without retrying.
      rc=$?
      return "$rc"
    fi
  done

  echo "ERROR: could not update $(claims_ref_name) ref after ${max_attempts} attempts (concurrent contention)" >&2
  return 1
}
