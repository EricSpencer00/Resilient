#!/usr/bin/env bash
# Mint a new ticket with a fresh RES-N id and a kebab-slug filename.
# Usage: .board/scripts/new-ticket.sh "Short title here"
set -euo pipefail

cd "$(dirname "$0")/../.."

title="${1:?Usage: new-ticket.sh \"Short title\"}"

# Next id: highest existing RES-N across all ticket states + 1.
next_id() {
    local max
    max=$(find .board/tickets -maxdepth 2 -name 'RES-*.md' -print 2>/dev/null \
        | sed -E 's@.*/RES-0*([0-9]+)-.*@\1@' \
        | sort -n | tail -1 || true)
    if [[ -z "${max:-}" ]]; then
        echo 1
    else
        echo $((max + 1))
    fi
}

id=$(next_id)
id_padded=$(printf "RES-%03d" "$id")
slug=$(echo "$title" \
    | tr '[:upper:]' '[:lower:]' \
    | sed -E 's/[^a-z0-9]+/-/g' \
    | sed -E 's/^-+|-+$//g' \
    | cut -c1-50)

path=".board/tickets/OPEN/${id_padded}-${slug}.md"
today=$(date +%Y-%m-%d)

cat > "$path" <<EOF
---
id: ${id_padded}
title: ${title}
state: OPEN
priority: P2
goalpost: TBD
created: ${today}
owner: executor
---

## Summary
TBD — fill me in.

## Acceptance criteria
- TBD

## Notes
- TBD

## Log
- ${today} created by manager
EOF

echo "Created: $path"
