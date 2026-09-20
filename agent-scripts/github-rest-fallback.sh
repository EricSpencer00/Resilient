#!/usr/bin/env bash
# Small REST fallbacks for guarded PR mutations.

github_graphql_rate_limited_output() {
  local output_file="$1"
  grep -Eiq 'API[[:space:]]+rate[[:space:]]+limit|secondary[[:space:]]+rate[[:space:]]+limit|rate[[:space:]-]?limit' "$output_file"
}

github_rest_find_open_pr() {
  local branch="$1"
  gh api "repos/{owner}/{repo}/pulls" \
    -f state=open -f "head=$branch" --jq '.[0].number // empty'
}

github_rest_pr_body() {
  local pr="$1"
  gh api "repos/{owner}/{repo}/pulls/$pr" --jq '.body // ""'
}

github_rest_pr_head_ref() {
  local pr="$1"
  gh api "repos/{owner}/{repo}/pulls/$pr" --jq '.head.ref // ""'
}

github_rest_mark_pr_ready() {
  local pr="$1"
  local draft

  # GitHub's documented REST update endpoint does not expose a
  # draft-to-ready operation.  Probe the current state first so an already
  # ready PR remains idempotent, then try the compatibility write and verify
  # the postcondition instead of treating an ignored field as success.
  draft="$(gh api "repos/{owner}/{repo}/pulls/$pr" --jq '.draft')" || return 1
  if [[ "$draft" == "false" ]]; then
    return 0
  fi

  gh api --method PATCH "repos/{owner}/{repo}/pulls/$pr" -f draft=false >/dev/null || return 1
  draft="$(gh api "repos/{owner}/{repo}/pulls/$pr" --jq '.draft')" || return 1
  if [[ "$draft" != "false" ]]; then
    echo "REST API did not convert PR #$pr from draft to ready; retry with gh pr ready when GraphQL is available." >&2
    return 1
  fi
}

github_rest_add_pr_label() {
  local pr="$1"
  local label="$2"
  local color="$3"
  local description="$4"

  gh api --method POST "repos/{owner}/{repo}/labels" \
    -f "name=$label" -f "color=$color" -f "description=$description" \
    >/dev/null 2>&1 || true
  printf '{"labels":["%s"]}\n' "$label" |
    gh api --method POST "repos/{owner}/{repo}/issues/$pr/labels" --input - \
      >/dev/null
}

github_rest_comment_pr() {
  local pr="$1"
  local body="$2"
  gh api --method POST "repos/{owner}/{repo}/issues/$pr/comments" \
    -f "body=$body" >/dev/null
}

github_mark_pr_ready() {
  local pr="$1"
  local output_file
  local status
  output_file="$(mktemp "${TMPDIR:-/tmp}/resilient-pr-ready.XXXXXX")"

  if gh pr ready "$pr" >"$output_file" 2>&1; then
    status=0
  else
    status=$?
  fi

  cat "$output_file"
  if (( status != 0 )) && grep -Eiq 'already[[:space:]]+ready[[:space:]]+for[[:space:]]+review' "$output_file"; then
    status=0
  elif (( status != 0 )) && github_graphql_rate_limited_output "$output_file"; then
    echo "GraphQL mutation unavailable; retrying ready transition through REST." >&2
    if github_rest_mark_pr_ready "$pr"; then
      status=0
    fi
  fi
  rm -f "$output_file"
  return "$status"
}

github_add_pr_label() {
  local pr="$1"
  local label="$2"
  local color="$3"
  local description="$4"
  local output_file
  local status
  output_file="$(mktemp "${TMPDIR:-/tmp}/resilient-pr-label.XXXXXX")"

  if gh pr edit "$pr" --add-label "$label" >"$output_file" 2>&1; then
    status=0
  else
    status=$?
  fi

  if (( status != 0 )) && github_graphql_rate_limited_output "$output_file"; then
    echo "GraphQL label mutation unavailable; retrying $label through REST." >&2
    if github_rest_add_pr_label "$pr" "$label" "$color" "$description"; then
      status=0
    fi
  fi
  cat "$output_file"
  rm -f "$output_file"
  return "$status"
}

github_comment_pr() {
  local pr="$1"
  local body="$2"
  local output_file
  local status
  output_file="$(mktemp "${TMPDIR:-/tmp}/resilient-pr-comment.XXXXXX")"

  if gh pr comment "$pr" --body "$body" >"$output_file" 2>&1; then
    status=0
  else
    status=$?
  fi

  if (( status != 0 )) && github_graphql_rate_limited_output "$output_file"; then
    echo "GraphQL comment mutation unavailable; retrying through REST." >&2
    if github_rest_comment_pr "$pr" "$body"; then
      status=0
    fi
  fi
  cat "$output_file"
  rm -f "$output_file"
  return "$status"
}
