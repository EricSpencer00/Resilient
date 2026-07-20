#!/usr/bin/env bash
# RES-3967: manual/local MCP HTTP request-latency report.
#
# The Live MCP Server initiative (#3934) states a <2s per-request
# success metric. `resilient/tests/mcp_latency_smoke.rs` enforces that
# SLA in CI (median over N=20 samples per tool, generous margin so a
# noisy shared runner can't flake it). This script is the sibling
# manual/local artifact: it spawns the real `rz mcp --http-port`
# binary, sends a batch of requests per representative tool call, and
# reports min/median/p95/max so a human (or a future perf-gate PR) can
# see the actual distribution, not just a pass/fail.
#
# Usage:
#   ./benchmarks/mcp_latency/run.sh [SAMPLES]
#
# Output: a single JSON object on stdout, e.g.
#   { "health_p50_ms": 3.1, "health_p95_ms": 5.2, "health_max_ms": 6.0,
#     "rz_parse_p50_ms": 4.8, "rz_parse_p95_ms": 9.1, "rz_parse_max_ms": 12.3,
#     "rz_typecheck_p50_ms": 5.9, ...,
#     "rz_run_p50_ms": 7.2, ...,
#     "samples": 30, "sla_ms": 2000, "system": "Darwin arm64",
#     "date": "2026-07-19T00:00:00Z" }
#
# Exit 0 on success. Exits non-zero (with a message on stderr) if any
# tool's p50 fails the 2s SLA — same threshold the CI smoke test
# enforces, surfaced here for local iteration.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$REPO_ROOT"

SAMPLES="${1:-30}"
RES_DIR=resilient
RES="$RES_DIR/target/release/rz"
CARGO_TARGET_DIR_OPT="${CARGO_TARGET_DIR:-$RES_DIR/target}"
RES="$CARGO_TARGET_DIR_OPT/release/rz"

if [[ ! -x "$RES" ]]; then
    (cd "$RES_DIR" && cargo build --release --locked --quiet)
fi

if ! command -v jq >/dev/null 2>&1; then
    echo "error: jq is required" >&2
    exit 1
fi

PORT=$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1]); s.close()')
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"; [[ -n "${SERVER_PID:-}" ]] && kill "$SERVER_PID" 2>/dev/null || true' EXIT

"$RES" mcp --http-port "127.0.0.1:$PORT" >"$TMP/server.log" 2>&1 &
SERVER_PID=$!

# Wait for the server to accept connections (bounded deadline).
deadline=$((SECONDS + 10))
until curl -s -o /dev/null "http://127.0.0.1:$PORT/health"; do
    if (( SECONDS > deadline )); then
        echo "error: server on port $PORT never became ready" >&2
        cat "$TMP/server.log" >&2
        exit 1
    fi
    sleep 0.1
done

SAMPLE_SOURCE='fn add(a: int, b: int) -> int { return a + b; } fn main() -> int { let total: int = 0; let i: int = 0; while i < 50 { total = add(total, i); i = i + 1; } return total; } main();'

body_for_tool() {
    jq -n --arg tool "$1" --arg source "$SAMPLE_SOURCE" '{tool: $tool, input: {source: $source}}'
}

PARSE_BODY=$(body_for_tool rz_parse)

percentile() {
    # $1 = sorted (ascending, one ms float per line) file, $2 = percentile (0-100)
    local file="$1" pct="$2"
    local n
    n=$(wc -l < "$file")
    local idx=$(( (n * pct + 99) / 100 ))
    (( idx < 1 )) && idx=1
    (( idx > n )) && idx=n
    sed -n "${idx}p" "$file"
}

measure() {
    local label="$1" method="$2" path="$3" body="$4"
    local out="$TMP/$label.txt"
    : > "$out"
    # Warm up.
    if [[ "$method" == "GET" ]]; then
        curl -s -o /dev/null "http://127.0.0.1:$PORT$path" || true
    else
        curl -s -o /dev/null -X POST -H 'Content-Type: application/json' -d "$body" "http://127.0.0.1:$PORT$path" || true
    fi
    for ((i = 0; i < SAMPLES; i++)); do
        local ms
        if [[ "$method" == "GET" ]]; then
            ms=$(curl -s -o /dev/null -w '%{time_total}' "http://127.0.0.1:$PORT$path")
        else
            ms=$(curl -s -o /dev/null -w '%{time_total}' -X POST -H 'Content-Type: application/json' -d "$body" "http://127.0.0.1:$PORT$path")
        fi
        echo "$ms * 1000" | bc >> "$out"
    done
    sort -n "$out" -o "$out"
}

measure health GET /health ""
measure rz_parse POST /mcp/call "$PARSE_BODY"

RZ_TYPECHECK_BODY=$(body_for_tool rz_typecheck)
RZ_RUN_BODY=$(body_for_tool rz_run)
measure rz_typecheck POST /mcp/call "$RZ_TYPECHECK_BODY"
measure rz_run POST /mcp/call "$RZ_RUN_BODY"

sys="$(uname -s) $(uname -m)"
date_iso="$(date -u +%Y-%m-%dT%H:%M:%SZ)"

sla_ms=2000
fail=0
report="{"
for label in health rz_parse rz_typecheck rz_run; do
    p50=$(percentile "$TMP/$label.txt" 50)
    p95=$(percentile "$TMP/$label.txt" 95)
    max=$(tail -n1 "$TMP/$label.txt")
    report="$report\"${label}_p50_ms\": $p50, \"${label}_p95_ms\": $p95, \"${label}_max_ms\": $max, "
    if (( $(echo "$p50 > $sla_ms" | bc) )); then
        echo "warning: $label p50 ${p50}ms exceeds ${sla_ms}ms SLA" >&2
        fail=1
    fi
done
report="$report\"samples\": $SAMPLES, \"sla_ms\": $sla_ms, \"system\": \"$sys\", \"date\": \"$date_iso\"}"

echo "$report" | jq .

exit $fail
