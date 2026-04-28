#!/bin/bash
# Verify that every builtin in resilient/src/main.rs appears in docs/STDLIB.md

set -e

STDLIB_FILE="docs/STDLIB.md"
MAIN_FILE="resilient/src/main.rs"

# Extract all builtin names from the BUILTINS const array
BUILTINS=$(awk '/const BUILTINS: &\[/,/^\];$/' "$MAIN_FILE" | grep '("' | cut -d'"' -f2 | sort)

if [ -z "$BUILTINS" ]; then
    echo "❌ Could not extract builtins from $MAIN_FILE"
    exit 1
fi

MISSING=()
for builtin in $BUILTINS; do
    # Check if the builtin name appears in the STDLIB.md file (heading, code, or text)
    # Allow for heading format like "### `builtin`" or "`builtin_*`" or just "builtin" in code/text
    if ! grep -q "\`$builtin\`" "$STDLIB_FILE"; then
        MISSING+=("$builtin")
    fi
done

if [ ${#MISSING[@]} -gt 0 ]; then
    echo "❌ The following builtins are NOT documented in $STDLIB_FILE:"
    for builtin in "${MISSING[@]}"; do
        echo "  - $builtin"
    done
    exit 1
fi

TOTAL=$(echo "$BUILTINS" | wc -w)
echo "✓ All $TOTAL builtins are documented in $STDLIB_FILE"
exit 0
