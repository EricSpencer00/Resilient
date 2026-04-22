#!/usr/bin/env bash
# check-ticket-ids.sh — verify no two board tickets share a RES-NNN id.
#
# Scans .board/tickets/OPEN/, .board/tickets/IN_PROGRESS/, and
# .board/tickets/DONE/ for `id: RES-NNN` front-matter fields and
# reports any duplicates.
#
# Exits 0 when all IDs are unique, 1 when a collision is found.
#
# Usage:
#   scripts/check-ticket-ids.sh
#   # or from a pre-commit hook:
#   .git/hooks/pre-commit

set -euo pipefail

BOARD_DIR="$(cd "$(dirname "$0")/.." && pwd)/.board/tickets"

if [[ ! -d "$BOARD_DIR" ]]; then
    echo "check-ticket-ids: board directory not found: $BOARD_DIR" >&2
    exit 1
fi

# Collect every `id: RES-NNN` value from all three queues.
# Output format: "<id> <file>" (one entry per ticket file).
mapfile -t ID_LINES < <(
    grep -r --include="*.md" "^id: RES-" \
        "$BOARD_DIR/OPEN/" \
        "$BOARD_DIR/IN_PROGRESS/" \
        "$BOARD_DIR/DONE/" \
        2>/dev/null \
    | sed 's|:id: RES-| RES-|'
)

TOTAL=${#ID_LINES[@]}

if [[ $TOTAL -eq 0 ]]; then
    echo "check-ticket-ids: no ticket files found — nothing to check."
    exit 0
fi

# Build a list of IDs only, then look for duplicates.
DUPLICATES=$(
    printf '%s\n' "${ID_LINES[@]}" \
    | awk '{print $2}' \
    | sort \
    | uniq -d
)

if [[ -n "$DUPLICATES" ]]; then
    echo "check-ticket-ids: FAIL — duplicate ticket IDs detected:" >&2
    while IFS= read -r dup_id; do
        echo "  $dup_id" >&2
        printf '%s\n' "${ID_LINES[@]}" \
        | grep " $dup_id$" \
        | awk '{print "    " $1}' >&2
    done <<< "$DUPLICATES"
    echo >&2
    echo "check-ticket-ids: Assign unique IDs with scripts/new-ticket.sh before committing." >&2
    exit 1
fi

echo "check-ticket-ids: OK — all $TOTAL ticket IDs are unique."
