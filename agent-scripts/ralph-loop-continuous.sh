#!/usr/bin/env bash
# ralph-loop-continuous.sh — continuous improvement loop with context checkpoint handling.
#
# This wrapper runs ralph-loop in batches, waiting between batches to allow
# the harness to compress context if needed. Use this for truly long-running
# autonomous improvement sessions.
#
# Usage:
#   agent-scripts/ralph-loop-continuous.sh              # loop until no more tickets
#   agent-scripts/ralph-loop-continuous.sh --max-batches 10  # stop after 10 batches
#   agent-scripts/ralph-loop-continuous.sh --dry-run    # plan only
#
# Environment:
#   RALPH_LOOP_CHECKPOINT_INTERVAL — tickets per batch (default 5)
#   RALPH_LOOP_BATCH_DELAY — seconds between batches (default 2)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CHECKPOINT_INTERVAL="${RALPH_LOOP_CHECKPOINT_INTERVAL:-5}"
BATCH_DELAY="${RALPH_LOOP_BATCH_DELAY:-2}"
MAX_BATCHES=""
DRY_RUN=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --max-batches) MAX_BATCHES="$2"; shift 2 ;;
    --dry-run) DRY_RUN=1; shift ;;
    --checkpoint-interval) CHECKPOINT_INTERVAL="$2"; shift 2 ;;
    --batch-delay) BATCH_DELAY="$2"; shift 2 ;;
    *) echo "unknown flag: $1" >&2; exit 2 ;;
  esac
done

log() { printf '[ralph-loop-continuous %s] %s\n' "$(date +%H:%M:%S)" "$*"; }

BATCH_COUNT=0
while true; do
  if [ -n "$MAX_BATCHES" ] && (( BATCH_COUNT >= MAX_BATCHES )); then
    log "max batches ($MAX_BATCHES) reached; stopping"
    break
  fi

  BATCH_COUNT=$(( BATCH_COUNT + 1 ))
  log "starting batch $BATCH_COUNT (max $CHECKPOINT_INTERVAL tickets per batch)"

  if (( DRY_RUN )); then
    bash "$SCRIPT_DIR/ralph-loop.sh" --checkpoint-interval "$CHECKPOINT_INTERVAL" --dry-run
  else
    bash "$SCRIPT_DIR/ralph-loop.sh" --checkpoint-interval "$CHECKPOINT_INTERVAL" || {
      EXIT_CODE=$?
      if (( EXIT_CODE != 0 )); then
        log "batch $BATCH_COUNT failed with exit code $EXIT_CODE; stopping"
        exit "$EXIT_CODE"
      fi
    }
  fi

  # Check if there are more tickets
  if ! bash "$SCRIPT_DIR/pick-ticket.sh" &>/dev/null; then
    log "no more tickets; improvement loop complete"
    break
  fi

  log "batch $BATCH_COUNT complete; waiting ${BATCH_DELAY}s before next batch..."
  sleep "$BATCH_DELAY"
done

log "continuous loop finished after $BATCH_COUNT batch(es)"
