#!/usr/bin/env bash
# install-hooks.sh — install the board pre-commit hook into .git/hooks/.
#
# Run once after cloning:
#   scripts/install-hooks.sh

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SRC="$ROOT/scripts/pre-commit"
DST="$ROOT/.git/hooks/pre-commit"

if [[ ! -f "$SRC" ]]; then
    echo "install-hooks: source hook not found: $SRC" >&2
    exit 1
fi

cp "$SRC" "$DST"
chmod +x "$DST"
echo "install-hooks: installed pre-commit hook → $DST"
