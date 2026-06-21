#!/usr/bin/env bash
# ralph-loop.sh — launch the high-leverage improvement loop.
#
# This is a thin wrapper around orchestrator.sh that prepends the reusable
# Ralph prompt to each ticket prompt and checkpoints every 5 iterations
# to avoid context overload in long-running sessions.
#
# Context checkpointing:
#   - In --loop mode, the orchestrator runs 5 iterations, then exits.
#   - To continue the loop, invoke ralph-loop.sh again.
#   - Use `--checkpoint-interval N` to adjust the checkpoint frequency.
#
# Usage:
#   agent-scripts/ralph-loop.sh                # 5 iterations, then checkpoint
#   agent-scripts/ralph-loop.sh --loop         # same as above
#   agent-scripts/ralph-loop.sh --n 1          # one iteration
#   agent-scripts/ralph-loop.sh --checkpoint-interval 10  # checkpoint every 10
#   agent-scripts/ralph-loop.sh --dry-run      # plan only

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(git rev-parse --show-toplevel)"

if [[ $# -eq 0 ]]; then
  set -- --loop
fi

export AGENT_LOOP_PROMPT_FILE="${AGENT_LOOP_PROMPT_FILE:-$REPO_ROOT/.board/prompts/ralph-loop.md}"
exec "$SCRIPT_DIR/orchestrator.sh" "$@"
