#!/usr/bin/env bash
# Manager ralph loop — runs `claude -p` with the manager prompt in a while loop.
#
# Safety:
#   - Stays in the Resilient repo.
#   - Capped iterations (env MAX_ITERS, default 20).
#   - Interval between iterations (env SLEEP_SECS, default 90).
#   - Logs all output to .board/logs/manager.log.
#
# Kill switch: create a file `.board/STOP` and the loop exits cleanly after
# the current iteration.

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"

LOG=".board/logs/manager.log"
PROMPT_FILE=".board/prompts/manager.md"
MAX_ITERS="${MAX_ITERS:-20}"
SLEEP_SECS="${SLEEP_SECS:-90}"

mkdir -p .board/logs
touch "$LOG"

echo "=== manager loop start $(date) (max=${MAX_ITERS}, sleep=${SLEEP_SECS}s) ===" >> "$LOG"

for i in $(seq 1 "$MAX_ITERS"); do
    if [[ -f .board/STOP ]]; then
        echo "--- manager: STOP file detected, exiting at iter $i ---" >> "$LOG"
        break
    fi

    echo "--- manager iter $i start $(date) ---" >> "$LOG"

    claude -p \
        --permission-mode bypassPermissions \
        --model opus \
        --add-dir "$ROOT" \
        "$(cat "$PROMPT_FILE")" \
        >> "$LOG" 2>&1 || echo "(manager iter $i exited non-zero)" >> "$LOG"

    echo "--- manager iter $i end $(date) ---" >> "$LOG"

    if [[ $i -lt $MAX_ITERS ]]; then
        sleep "$SLEEP_SECS"
    fi
done

echo "=== manager loop end $(date) ===" >> "$LOG"
