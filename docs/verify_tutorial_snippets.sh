#!/usr/bin/env bash
# RES-207: extract every ```resilient code block from
# docs/tutorial/*.md and run each one through the rz
# binary. Exit 0 when every snippet runs to completion (exit 0
# from the compiler); non-zero on the first failure.
#
# Intended to be wired into CI as a gate on doc changes — if a
# tutorial snippet stops working, the docs change fails.
#
# Portability: written for bash 3.2 (macOS system bash) without
# `mapfile` / `readarray` / other bash-4 idioms.
#
# Assumptions:
# - `rz` binary is on PATH OR pointed at via $RESILIENT_BIN.
#   Defaults to `resilient/target/release/rz` relative to
#   repo root, then falls back to `rz` on PATH.
# - Code blocks are fenced with triple-backticks followed by the
#   language tag `resilient` (case-sensitive). Other fences
#   (```bash, ```rust, etc.) are treated as documentation-only
#   and skipped.

set -eu

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"

BIN=${RESILIENT_BIN:-}
if [ -z "$BIN" ]; then
    if [ -x resilient/target/release/rz ]; then
        BIN=resilient/target/release/rz
    elif [ -x resilient/target/debug/rz ]; then
        BIN=resilient/target/debug/rz
    elif command -v rz >/dev/null 2>&1; then
        BIN=rz
    else
        echo "error: rz binary not found. Run 'cd resilient && cargo build' or set RESILIENT_BIN." >&2
        exit 2
    fi
fi

echo "Using rz binary: $BIN"

TUTORIAL_DIR=docs/tutorial
if [ ! -d "$TUTORIAL_DIR" ]; then
    echo "error: $TUTORIAL_DIR missing" >&2
    exit 2
fi

TMP_DIR=$(mktemp -d)
trap 'rm -rf "$TMP_DIR"' EXIT

total=0
failed=0
failures=""

for md in $(ls "$TUTORIAL_DIR"/*.md | sort); do
    echo
    echo "=== $md ==="

    # Split the markdown into one file per ```resilient block.
    # awk writes each block to $TMP_DIR/$(basename md)_<n>.rs.
    tag=$(basename "$md" .md)
    awk -v tag="$tag" -v out="$TMP_DIR" '
        /^```resilient$/  { snippet++; path=out"/"tag"_"snippet".rs"; in_block=1; next }
        in_block && /^```$/ { in_block=0; close(path); next }
        in_block          { print > path }
    ' "$md"

    # Execute each extracted block in order.
    for snippet in $(ls "$TMP_DIR"/"$tag"_*.rs 2>/dev/null | sort); do
        total=$((total + 1))
        num=$(basename "$snippet" .rs | sed "s/^${tag}_//")
        if "$BIN" --seed 0 "$snippet" >/dev/null 2>&1; then
            echo "  snippet $num: OK"
        else
            rc=$?
            echo "  snippet $num: FAIL (exit $rc)"
            echo "    (content preview)"
            head -5 "$snippet" | sed 's/^/      /'
            failed=$((failed + 1))
            failures="$failures $md:$num"
        fi
    done
done

echo
echo "==========================="
echo "Tutorial snippet verification"
echo "  total:  $total"
echo "  failed: $failed"
if [ "$failed" -gt 0 ]; then
    echo "  failures:$failures"
    exit 1
fi
echo "==========================="
echo "All tutorial snippets ran cleanly."
