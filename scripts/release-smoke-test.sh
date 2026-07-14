#!/usr/bin/env bash
# RES-3979: release smoke test for Z3-enabled `rz` binaries.
#
# Building with `--features z3,z3-static` is silent about whether the
# resulting binary can actually discharge an SMT-only proof obligation
# — a binary that links but never calls into Z3 correctly would still
# print `Unknown`/leave the clause for runtime, exactly like a plain
# no-z3 build, and nobody would notice until a user complained. This
# script runs `rz --audit` against a canned fixture whose only
# `requires` clause is a universal tautology over a free variable
# (`x + 0 == x`) that the hand-rolled folder (RES-060..065) cannot
# reduce but that stock Z3 proves instantly — see
# `.github/fixtures/z3_release_smoke.rz`. A real Z3-backed build must
# report it as "proven by Z3 (SMT)"; anything else is a release-time
# regression.
#
# Usage:
#   scripts/release-smoke-test.sh <path-to-rz-binary>
set -euo pipefail

BIN="${1:?usage: release-smoke-test.sh <path-to-rz-binary>}"
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
FIXTURE="${REPO_ROOT}/.github/fixtures/z3_release_smoke.rz"

[ -x "$BIN" ] || { echo "error: $BIN is not an executable file" >&2; exit 1; }
[ -f "$FIXTURE" ] || { echo "error: fixture not found at $FIXTURE" >&2; exit 1; }

OUTPUT="$("$BIN" --audit "$FIXTURE" 2>&1)" || {
    echo "error: '$BIN --audit $FIXTURE' exited non-zero" >&2
    echo "$OUTPUT" >&2
    exit 1
}

echo "$OUTPUT"

# Strip ANSI color codes (the audit report colorizes counts) before
# grepping the "of which proven by Z3 (SMT): N" line.
CLEANED="$(printf '%s\n' "$OUTPUT" | sed -E 's/\x1B\[[0-9;]*[A-Za-z]//g')"
COUNT="$(printf '%s\n' "$CLEANED" | grep -oE 'proven by Z3 \(SMT\):[[:space:]]*[0-9]+' | grep -oE '[0-9]+$' || true)"

if [ -z "$COUNT" ] || [ "$COUNT" -lt 1 ]; then
    echo "" >&2
    echo "FAIL: expected 'rz --audit' to report at least one clause" >&2
    echo "      'proven by Z3 (SMT)', got: '${COUNT:-<line not printed>}'." >&2
    echo "      This binary was built with --features z3,z3-static but" >&2
    echo "      did not actually discharge the obligation via Z3 — the" >&2
    echo "      shipped release binary would silently regress to" >&2
    echo "      Unknown/runtime-deferred for every user, exactly like" >&2
    echo "      today's no-z3 binaries." >&2
    exit 1
fi

echo ""
echo "OK: rz --audit reports ${COUNT} clause(s) proven by Z3 (SMT)."
