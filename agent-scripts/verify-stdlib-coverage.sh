#!/bin/bash
#
# Verify that every builtin in resilient/src/main.rs::BUILTINS
# is documented in docs/STDLIB.md.
#
# Exit code: 0 if all builtins are documented, 1 if any are missing.

set -euo pipefail

STDLIB_FILE="${1:-docs/STDLIB.md}"
MAIN_FILE="${2:-resilient/src/main.rs}"

if [[ ! -f "$STDLIB_FILE" ]]; then
    echo "FAIL  $STDLIB_FILE does not exist" >&2
    exit 1
fi

if [[ ! -f "$MAIN_FILE" ]]; then
    echo "FAIL  $MAIN_FILE does not exist" >&2
    exit 1
fi

# Extract builtin names from BUILTINS const in main.rs
# Pattern: ("builtin_name", builtin_function),
# Extract the first group (builtin_name in quotes)
# Only match lines with both ( and , to avoid matching unrelated lines
BUILTINS=$(sed -n 's/^[[:space:]]*("\([^"]*\)",[[:space:]]*builtin.*/\1/p' "$MAIN_FILE" || true)

if [[ -z "$BUILTINS" ]]; then
    echo "FAIL  No builtins found in $MAIN_FILE" >&2
    exit 1
fi

# Check each builtin is documented in STDLIB.md
MISSING=()
for builtin in $BUILTINS; do
    # Look for the builtin name in markdown headers like:
    # ### `builtin_name`
    # or in code blocks like:
    # | `builtin_name` | ... |
    if ! grep -q "\`$builtin\`" "$STDLIB_FILE"; then
        MISSING+=("$builtin")
    fi
done

if [[ ${#MISSING[@]} -gt 0 ]]; then
    echo "FAIL  The following builtins are missing from $STDLIB_FILE:" >&2
    printf '       %s\n' "${MISSING[@]}" >&2
    exit 1
fi

# Count documented builtins
DOCUMENTED=$(grep -c "^### \`" "$STDLIB_FILE" || true)

echo "PASS  All $DOCUMENTED builtins documented in $STDLIB_FILE"
exit 0
