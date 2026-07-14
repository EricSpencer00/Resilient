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
# `.json` files (e.g. agent-scripts/file-claims.json) are NOT line
# structures — textually concatenating two object-members drops the
# comma separator and yields invalid JSON. For those, the two sides are
# each reconstructed into a whole document, parsed as JSON, and merged
# at the data level (union of keys). See the `.json` branch below.
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
    # RES-929: lib.rs is the real extension-block file (1.8 MB);
    # main.rs is a 463-byte binary entry. Keep main.rs allowlisted
    # for legacy compatibility; lib.rs is now the primary path.
    "resilient/src/lib.rs"
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

    if [[ "$file" == *.json ]]; then
        # RES-3931: data-level merge for JSON. Reconstruct the `ours`
        # and `theirs` documents by resolving every conflict hunk to a
        # single side, parse each as JSON (each side was a committed,
        # valid document), then deep-union the objects. On a key
        # collision the entry with the later `claimed_at` wins, falling
        # back to `ours`. Output is always valid, pretty-printed JSON.
        python3 - "$file" <<'PY'
import json, pathlib, re, sys

path = pathlib.Path(sys.argv[1])
text = path.read_text()
pattern = re.compile(
    r'<<<<<<< [^\n]*\n(.*?)^=======\n(.*?)^>>>>>>> [^\n]*\n',
    re.DOTALL | re.MULTILINE,
)

if not pattern.search(text):
    # No conflict markers — leave a clean file untouched.
    if '<<<<<<<' in text or '>>>>>>>' in text:
        sys.exit(f"ERROR: malformed conflict markers in {path}")
    print(f"clean: {path}")
    sys.exit(0)

ours = pattern.sub(lambda m: m.group(1), text)
theirs = pattern.sub(lambda m: m.group(2), text)
for side in (ours, theirs):
    if '<<<<<<<' in side or '>>>>>>>' in side or '\n=======\n' in side:
        sys.exit(f"ERROR: unresolved markers remain in {path}")

try:
    ours_doc = json.loads(ours)
    theirs_doc = json.loads(theirs)
except json.JSONDecodeError as exc:
    sys.exit(f"ERROR: a side of {path} is not valid JSON after conflict "
             f"split — resolve by hand.\n{exc}")


def newer(a, b):
    """Return the claim entry that should win on a key collision."""
    ta = a.get("claimed_at", "") if isinstance(a, dict) else ""
    tb = b.get("claimed_at", "") if isinstance(b, dict) else ""
    return b if tb > ta else a


def is_entry(d):
    # A claim entry is atomic (do not field-merge it): the more recent
    # claim wins as a whole. The container objects ("claims" map, root)
    # carry neither of these keys.
    return isinstance(d, dict) and ("claimed_at" in d or "branch" in d)


def deep_union(a, b):
    if isinstance(a, dict) and isinstance(b, dict):
        out = dict(a)
        for k, v in b.items():
            if k not in out:
                out[k] = v
            elif isinstance(out[k], dict) and isinstance(v, dict) \
                    and not (is_entry(out[k]) or is_entry(v)):
                # Container object (e.g. the "claims" map): union recursively.
                out[k] = deep_union(out[k], v)
            else:
                # Claim entry or scalar collision: keep the newer whole value.
                out[k] = newer(out[k], v)
        return out
    # Non-dict leaf conflict: keep `ours`.
    return a


merged = deep_union(ours_doc, theirs_doc)
path.write_text(json.dumps(merged, indent=2) + "\n")
print(f"resolved: {path}")
PY
        continue
    fi

    python3 - "$file" <<'PY'
import pathlib, re, sys, subprocess, tempfile, os
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
if out == text:
    print(f"clean: {path}")
    sys.exit(0)

# Write resolved content and verify with rustfmt before committing.
# If rustfmt can't parse the file, the resolution was syntactically
# invalid and must be done by hand.
with tempfile.NamedTemporaryFile(mode='w', suffix='.rs', delete=False) as tmp:
    tmp.write(out)
    tmp_path = tmp.name
try:
    result = subprocess.run(
        ['rustfmt', '--edition', '2021', '--check', tmp_path],
        capture_output=True, text=True
    )
    if result.returncode not in (0, 1):  # 0=ok, 1=would reformat, 2=parse error
        sys.exit(f"ERROR: rustfmt parse failure after auto-resolve of {path} — "
                 f"resolve this conflict by hand.\nrustfmt stderr: {result.stderr[:400]}")
except FileNotFoundError:
    pass  # rustfmt not available — skip check
finally:
    os.unlink(tmp_path)

path.write_text(out)
print(f"resolved: {path}")
PY
done
