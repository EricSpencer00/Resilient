#!/usr/bin/env bash
# check-stdlib-completeness.sh
#
# RES-321: Verify that every builtin function listed in resilient/src/main.rs
# BUILTINS array also appears in docs/STDLIB.md.
#
# Exit code 0: all builtins documented
# Exit code 1: missing builtins or files not found

set -euo pipefail

REPO_ROOT="$(git rev-parse --show-toplevel)"
cd "$REPO_ROOT"

MAIN_RS="resilient/src/main.rs"
STDLIB_MD="docs/STDLIB.md"

if [[ ! -f "$MAIN_RS" ]]; then
    echo "ERROR: $MAIN_RS not found" >&2
    exit 1
fi

if [[ ! -f "$STDLIB_MD" ]]; then
    echo "ERROR: $STDLIB_MD not found" >&2
    exit 1
fi

# Extract builtin names from the BUILTINS array.
# Format: ("name", builtin_fn) — extract only from the array section.
# Use sed to extract from the BUILTINS array and get quoted strings.
BUILTINS=$(sed -n '/^const BUILTINS:/,/^];/p' "$MAIN_RS" | \
    sed -n 's/.*("\([a-zA-Z_][a-zA-Z0-9_]*\)".*/\1/p' | \
    sort | uniq)

if [[ -z "$BUILTINS" ]]; then
    echo "WARNING: No builtins found in $MAIN_RS" >&2
    exit 0
fi

missing=()

for builtin in $BUILTINS; do
    # Check if the builtin appears in STDLIB.md.
    # Look for the builtin name in code snippets or headings.
    # Account for cases like "### \`as_int8\`, \`as_int16\`" where multiple
    # variants are grouped on one heading line.
    if ! grep -q "\`$builtin\`" "$STDLIB_MD"; then
        missing+=("$builtin")
    fi
done

if [[ ${#missing[@]} -gt 0 ]]; then
    echo "ERROR: The following builtins are in $MAIN_RS but NOT documented in $STDLIB_MD:" >&2
    printf '  %s\n' "${missing[@]}" >&2
    exit 1
fi

echo "✓ All builtins documented in $STDLIB_MD"
exit 0
