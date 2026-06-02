#!/usr/bin/env bash
# ralph-loop.sh — launch the high-leverage improvement loop.
#
# This is a thin wrapper around orchestrator.sh that prepends the reusable
# Ralph prompt to each ticket prompt and defaults to an unbounded loop.
#
# Usage:
#   agent-scripts/ralph-loop.sh                # continuous loop
#   agent-scripts/ralph-loop.sh --n 1          # one iteration
#   agent-scripts/ralph-loop.sh --dry-run      # plan only

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(git rev-parse --show-toplevel)"

if [[ $# -eq 0 ]]; then
  set -- --loop
fi

export AGENT_LOOP_PROMPT_FILE="${AGENT_LOOP_PROMPT_FILE:-$REPO_ROOT/.board/prompts/ralph-loop.md}"
exec "$SCRIPT_DIR/orchestrator.sh" "$@"
