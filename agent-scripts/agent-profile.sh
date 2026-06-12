#!/usr/bin/env bash
# Shared defaults for autonomous agent loops.
#
# Keep this agent-neutral. Callers may export any value before launching:
#   AGENT_CMD="claude -p --permission-mode acceptEdits"
#   AGENT_CMD="codex exec"
#   AGENT_COAUTHOR_NAME="..."
#   AGENT_COAUTHOR_EMAIL="..."

REPO_ROOT="${REPO_ROOT:-${PRIMARY_ROOT:-$(git rev-parse --show-toplevel)}}"

: "${AGENT_DISPLAY_NAME:=Resilient Agent}"
: "${AGENT_GITHUB_USER:=}"
: "${AGENT_CMD:=${RESILIENT_IMPROVEMENT_AGENT_CMD:-codex exec}}"
: "${AGENT_COAUTHOR_NAME:=}"
: "${AGENT_COAUTHOR_EMAIL:=}"
: "${AGENT_LOOP_PROMPT_FILE:=$REPO_ROOT/.board/prompts/ralph-loop.md}"
: "${AGENT_BOT_LOGINS:=Copilot,Claude,claude,copilot-swe-agent,anthropic-code-agent,codex}"

export AGENT_DISPLAY_NAME
export AGENT_GITHUB_USER
export AGENT_CMD
export AGENT_COAUTHOR_NAME
export AGENT_COAUTHOR_EMAIL
export AGENT_LOOP_PROMPT_FILE
export AGENT_BOT_LOGINS
