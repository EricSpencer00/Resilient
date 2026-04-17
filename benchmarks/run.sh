#!/usr/bin/env bash
# Run all benchmarks via hyperfine and dump a markdown table.
#
# Usage:
#   ./benchmarks/run.sh
#
# Each benchmark file is expected to print the same answer regardless
# of language. Hyperfine handles warmup, multiple runs, and reports
# mean ± std dev.

set -euo pipefail

cd "$(dirname "$0")/.."

RES=resilient/target/release/resilient
if [[ ! -x "$RES" ]]; then
    echo "Building release binary..."
    (cd resilient && cargo build --release --quiet)
fi

# Make sure native baselines are built and up-to-date.
rustc -O benchmarks/fib/fib_native.rs -o benchmarks/fib/fib_native 2>/dev/null
rustc -O benchmarks/sum/sum_native.rs -o benchmarks/sum/sum_native 2>/dev/null

OUT=benchmarks/RESULTS.md
{
    echo "# Benchmark Results"
    echo
    echo "Hardware: $(uname -sm), $(sysctl -n machdep.cpu.brand_string 2>/dev/null || echo 'unknown CPU')"
    echo "Date: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
    echo "Resilient build: \`cargo build --release\` (default features)"
    echo
} > "$OUT"

bench() {
    local name="$1"
    shift
    echo "## $name" >> "$OUT"
    echo >> "$OUT"
    echo "Running: $name"
    hyperfine --warmup 2 --runs 5 --export-markdown - "$@" >> "$OUT"
    echo >> "$OUT"
}

bench "fib(25) — recursive Fibonacci" \
    --command-name "Resilient (interp)" "$RES benchmarks/fib/fib.rs" \
    --command-name "Python 3"           "python3 benchmarks/fib/fib.py" \
    --command-name "Node.js"            "node benchmarks/fib/fib.js" \
    --command-name "Lua"                "lua benchmarks/fib/fib.lua" \
    --command-name "Ruby"               "ruby benchmarks/fib/fib.rb" \
    --command-name "Rust (native -O)"   "./benchmarks/fib/fib_native"

bench "sum 1..100000 — while-loop accumulator" \
    --command-name "Resilient (interp)" "$RES benchmarks/sum/sum.rs" \
    --command-name "Python 3"           "python3 benchmarks/sum/sum.py" \
    --command-name "Node.js"            "node benchmarks/sum/sum.js" \
    --command-name "Lua"                "lua benchmarks/sum/sum.lua" \
    --command-name "Ruby"               "ruby benchmarks/sum/sum.rb" \
    --command-name "Rust (native -O)"   "./benchmarks/sum/sum_native"

# Contract overhead — Resilient-only. Same workload, with and without
# a `requires` clause that fires on every call. Measures the cost of
# runtime contract checking; an upper bound on what `--audit`-driven
# static discharge would save (see RES-068 future work).
bench "contract overhead — 100k safe_div calls" \
    --command-name "Resilient + requires"  "$RES benchmarks/contracts/with_contract.rs" \
    --command-name "Resilient (no contract)" "$RES benchmarks/contracts/no_contract.rs"

echo
echo "Done. Results in $OUT"
