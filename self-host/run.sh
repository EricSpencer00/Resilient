#!/usr/bin/env bash
# RES-196: driver for the self-hosted lexer prototype. Runs
# `self-host/lex.rs` through the Rust interpreter against
# `resilient/examples/hello.rs` and diffs the output against
# the committed snapshot `self-host/hello.tokens.txt`.
#
# Exit code:
#   0 — output matches the snapshot
#   1 — mismatch (prints a unified diff)
#   2 — couldn't find the resilient binary
#
# The script is NOT wired into CI per the ticket's Note — it's a
# manual sanity check until the self-hosted toolchain becomes
# load-bearing.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"

# Find the resilient binary. Prefer a debug build (that's what
# `cargo build` produces); fall back to release.
BIN=""
if [[ -x resilient/target/debug/resilient ]]; then
    BIN=resilient/target/debug/resilient
elif [[ -x resilient/target/release/resilient ]]; then
    BIN=resilient/target/release/resilient
else
    echo "error: no resilient binary found; run 'cd resilient && cargo build' first." >&2
    exit 2
fi

EXPECTED=self-host/hello.tokens.txt
TMPOUT=$(mktemp)
trap 'rm -f "$TMPOUT"' EXIT

# Strip noise the runtime prints on stderr (seed=… line) and the
# trailing "Program executed successfully" from stdout. The
# snapshot only carries the user-meaningful tokens.
"$BIN" self-host/lex.rs 2>/dev/null \
  | grep -v "^seed=" \
  | grep -v "^Program executed successfully$" \
  > "$TMPOUT"

if diff -u "$EXPECTED" "$TMPOUT"; then
    echo "self-host: token snapshot OK (lexed $(wc -l < "$TMPOUT" | tr -d ' ') tokens)"
    exit 0
else
    echo "self-host: token snapshot MISMATCH" >&2
    exit 1
fi
