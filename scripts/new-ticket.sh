#!/usr/bin/env bash
# new-ticket.sh — mint a new board ticket with the next unused RES-NNN id.
#
# Usage:
#   scripts/new-ticket.sh "Short imperative title under 60 chars"
#
# Creates  .board/tickets/OPEN/RES-NNN-kebab-title.md  then prints the
# path so the caller can open it in an editor.
#
# The next ID is derived by scanning every ticket file in OPEN/,
# IN_PROGRESS/, and DONE/ for `id: RES-NNN` front-matter and taking
# max(all_ids) + 1.  If no tickets exist, numbering starts at 1.

set -euo pipefail

if [[ $# -lt 1 || -z "$1" ]]; then
    echo "usage: scripts/new-ticket.sh \"Short title\"" >&2
    exit 1
fi

TITLE="$1"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BOARD_DIR="$ROOT/.board/tickets"
OPEN_DIR="$BOARD_DIR/OPEN"

mkdir -p "$OPEN_DIR"

# Find the highest existing RES-NNN id across all queues.
MAX_ID=$(
    grep -r --include="*.md" "^id: RES-" \
        "$BOARD_DIR/OPEN/" \
        "$BOARD_DIR/IN_PROGRESS/" \
        "$BOARD_DIR/DONE/" \
        2>/dev/null \
    | sed 's/.*RES-//' \
    | sort -n \
    | tail -1
)

if [[ -z "$MAX_ID" ]]; then
    NEXT_ID=1
else
    NEXT_ID=$(( MAX_ID + 1 ))
fi

ID="RES-${NEXT_ID}"

# Build a filesystem-safe slug from the title:
#   1. lowercase all characters
#   2. replace non-alphanumeric characters with hyphens
#   3. collapse multiple consecutive hyphens into one
#   4. strip leading and trailing hyphens
#   5. trim to 60 characters
SLUG=$(echo "$TITLE" \
    | tr '[:upper:]' '[:lower:]' \
    | sed 's/[^a-z0-9]/-/g' \
    | sed 's/-\+/-/g' \
    | sed 's/^-\|-$//g')
SLUG="${SLUG:0:60}"

FILENAME="${ID}-${SLUG}.md"
FILEPATH="$OPEN_DIR/$FILENAME"

if [[ -e "$FILEPATH" ]]; then
    echo "new-ticket: file already exists: $FILEPATH" >&2
    exit 1
fi

TODAY=$(date -u +%Y-%m-%d)

cat > "$FILEPATH" <<EOF
---
id: ${ID}
title: ${TITLE}
state: OPEN
priority: P2
goalpost: G0
created: ${TODAY}
owner: executor
---

## Summary

<!-- One paragraph. What's broken/missing, why it matters. -->

## Acceptance criteria

- <!-- Specific command to run, file/line change, or observable behaviour -->

## Notes

- <!-- Relevant file paths, known pitfalls, links to related tickets -->

## Log

- ${TODAY} created by new-ticket.sh
EOF

echo "$FILEPATH"
