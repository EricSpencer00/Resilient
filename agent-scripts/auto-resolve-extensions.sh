#!/usr/bin/env bash
# auto-resolve-extensions.sh
#
# Merge-conflict auto-resolver for append-only extension blocks.
#
# When two agents add independent entries to the same extension block
# (<EXTENSION_TOKENS>, <EXTENSION_KEYWORDS>, <EXTENSION_PASSES>) the
# conflict is always resolved by keeping *both* sides. This script does
# exactly that: for each file it collapses every
#
#     <<<<<<< ours
#     A
#     =======
#     B
#     >>>>>>> theirs
#
# hunk into `A` followed by `B`.
#
# Safe only for the core extension files listed below. DO NOT run this
# on arbitrary files — a real conflict in logic code must be resolved
# by hand.
#
# Usage:
#   agent-scripts/auto-resolve-extensions.sh <file> [<file> ...]
#
# Exits non-zero if any conflict markers remain after the pass.

set -euo pipefail

if [[ $# -eq 0 ]]; then
    echo "usage: $0 <file> [<file> ...]" >&2
    exit 2
fi

ALLOWED=(
    "resilient/src/main.rs"
    "resilient/src/typechecker.rs"
    "resilient/src/lexer_logos.rs"
    "agent-scripts/file-claims.json"
)

for file in "$@"; do
    ok=0
    for allowed in "${ALLOWED[@]}"; do
        [[ "$file" == *"$allowed" ]] && ok=1 && break
    done
    if [[ $ok -eq 0 ]]; then
        echo "refuse: $file is not in the append-only allowlist" >&2
        echo "  allowlist: ${ALLOWED[*]}" >&2
        exit 3
    fi

    if [[ ! -f "$file" ]]; then
        echo "skip: $file does not exist" >&2
        continue
    fi

    python3 - "$file" <<'PY'
import pathlib, re, sys
path = pathlib.Path(sys.argv[1])
text = path.read_text()
pattern = re.compile(
    r'<<<<<<< [^\n]*\n(.*?)^=======\n(.*?)^>>>>>>> [^\n]*\n',
    re.DOTALL | re.MULTILINE,
)
def merge(m):
    return m.group(1) + m.group(2)
out = pattern.sub(merge, text)
if '<<<<<<<' in out or '>>>>>>>' in out or '\n=======\n' in out:
    sys.exit(f"ERROR: unresolved markers remain in {path}")
if out != text:
    path.write_text(out)
    print(f"resolved: {path}")
else:
    print(f"clean: {path}")
PY
done
