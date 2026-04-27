#!/usr/bin/env bash
# orchestrator.sh — the long-running "grand agent" loop.
#
# One iteration:
#   1. pick-ticket.sh     → next agent-ready issue with no open PR
#   2. dispatch-agent.sh  → fresh worktree + branch + draft PR
#   3. claude -p          → run a sub-agent inside the worktree (non-interactive)
#   4. ready-or-bail.sh   → guardrail; PR → ready or stays draft w/ comment
#   5. loop
#
# Runs N iterations then exits (default 1). Set `--loop` for unbounded.
# Runs iterations serially by default; use `--parallel K` to fan out.
#
# Usage:
#   agent-scripts/orchestrator.sh                  # one ticket
#   agent-scripts/orchestrator.sh --n 5            # five sequential
#   agent-scripts/orchestrator.sh --n 3 --parallel 3
#   agent-scripts/orchestrator.sh --loop           # until no more tickets
#   agent-scripts/orchestrator.sh --dry-run        # plan only, no mutations
#
# Env:
#   AGENT_CMD — command used to run the sub-agent inside each worktree.
#     Default: `claude -p --permission-mode acceptEdits`. Override to plug in
#     other CLIs (e.g. `codex` or a mock for testing).

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(git rev-parse --show-toplevel)"

N=1
UNBOUNDED=0
PARALLEL=1
DRY_RUN=0
AGENT_CMD="${AGENT_CMD:-claude -p --permission-mode acceptEdits}"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --n) N="$2"; shift 2 ;;
    --loop) UNBOUNDED=1; shift ;;
    --parallel) PARALLEL="$2"; shift 2 ;;
    --dry-run) DRY_RUN=1; shift ;;
    *) echo "unknown flag: $1" >&2; exit 2 ;;
  esac
done

log() { printf '[orchestrator %s] %s\n' "$(date +%H:%M:%S)" "$*"; }

run_one() {
  local issue="$1"

  log "dispatching #${issue}"
  if (( DRY_RUN )); then
    bash "$SCRIPT_DIR/dispatch-agent.sh" --issue "$issue" --dry-run
    return 0
  fi

  local dispatch_out
  dispatch_out="$(bash "$SCRIPT_DIR/dispatch-agent.sh" --issue "$issue" 2>&1)" || {
    log "dispatch failed for #${issue}:"; echo "$dispatch_out"; return 1
  }
  local worktree
  worktree="$(printf '%s' "$dispatch_out" | awk -F': +' '/^Worktree/ {print $2; exit}')"
  local pr
  pr="$(printf '%s' "$dispatch_out" | awk -F'/pull/' '/Draft PR/ {print $2; exit}')"
  log "worktree=$worktree pr=$pr"
  "$SCRIPT_DIR/agent-handoff.sh" \
    --pr "$pr" \
    --issue "$issue" \
    --phase executor-starting \
    --status "sub-agent prompt prepared" \
    --worktree "$worktree" \
    --summary "The orchestrator is launching the executor. Resume from the issue body, branch diff, latest commits, CI status, and this handoff thread." >/dev/null || true

  local body_file="/tmp/issue-${issue}.md"
  gh issue view "$issue" --json title,body -q '.title + "\n\n" + .body' > "$body_file"

  local prompt_file="/tmp/agent-prompt-${issue}.md"
  cat > "$prompt_file" <<EOF
You are working on Resilient. A worktree is prepared at:
  ${worktree}
Branch is pushed and tracks origin. Draft PR is #${pr} with 'Closes #${issue}'.
The single existing commit is an empty claim commit — don't amend it, add new commits on top.

Your ticket (from GitHub):

$(cat "$body_file")

Rules (enforced post-hoc by agent-scripts/verify-scope.sh, which decides whether your PR gets marked ready):
  - Do NOT modify existing tests or .expected.txt files — only add new ones.
  - Do NOT introduce new \`unsafe\` blocks.
  - Do NOT edit .github/workflows/.
  - Do NOT bump major/minor dependency versions in Cargo.lock.
  - Keep the touched file count under 60.
  - cargo fmt, cargo clippy -D warnings, and cargo test must all pass.

Work in this order:
  1. cd ${worktree}
  2. Read CLAUDE.md, explore the relevant source files.
  3. Implement. Commit in logical chunks with 'RES-${issue}: ...' subject lines and a Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com> trailer.
  4. Push (the branch already tracks).
  5. Run agent-scripts/ready-or-bail.sh --pr ${pr}. If it fails, read the comment it posts, fix the issues, push again, re-run. Do NOT run 'gh pr ready' manually — let the guardrail decide.

When you're finished (guardrail green OR exhausted — don't loop forever on a stuck ticket), exit. Report under 300 words what you shipped and whether the PR is ready or left draft.
EOF

  log "launching sub-agent for #${issue}"
  (cd "$worktree" && $AGENT_CMD < "$prompt_file") || log "agent exited non-zero for #${issue}"

  # Best-effort: if the agent forgot, run the guardrail ourselves.
  (cd "$worktree" && bash "$SCRIPT_DIR/ready-or-bail.sh" --pr "$pr") || true

  "$SCRIPT_DIR/agent-handoff.sh" \
    --pr "$pr" \
    --issue "$issue" \
    --phase executor-finished \
    --status "orchestrator iteration complete" \
    --worktree "$worktree" \
    --summary "Executor process returned. Inspect the branch diff, latest PR comments, and guardrail report to resume." >/dev/null || true

  log "finished #${issue}"
}

pick_next() {
  local excl=("$@")
  local flags=()
  for e in "${excl[@]}"; do flags+=(--exclude "$e"); done
  bash "$SCRIPT_DIR/pick-ticket.sh" "${flags[@]}" 2>/dev/null | cut -f1
}

EXCLUDED=()
COUNT=0
while true; do
  if (( UNBOUNDED == 0 )) && (( COUNT >= N )); then break; fi

  if (( PARALLEL > 1 )); then
    PIDS=()
    BATCH=()
    for ((k=0; k<PARALLEL; k++)); do
      issue="$(pick_next "${EXCLUDED[@]}")"
      [ -z "$issue" ] && break
      EXCLUDED+=("$issue")
      BATCH+=("$issue")
      log "batch slot $k → #${issue}"
    done
    [ ${#BATCH[@]} -eq 0 ] && { log "no more tickets"; break; }
    for issue in "${BATCH[@]}"; do
      ( run_one "$issue" ) &
      PIDS+=($!)
    done
    for pid in "${PIDS[@]}"; do wait "$pid" || true; done
    COUNT=$(( COUNT + ${#BATCH[@]} ))
  else
    issue="$(pick_next "${EXCLUDED[@]}")"
    [ -z "$issue" ] && { log "no more tickets"; break; }
    EXCLUDED+=("$issue")
    run_one "$issue" || true
    COUNT=$(( COUNT + 1 ))
  fi
done

log "orchestrator finished — dispatched $COUNT ticket(s)"
