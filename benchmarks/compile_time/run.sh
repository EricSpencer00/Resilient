#!/usr/bin/env bash
# RES-1586: compile-time benchmark harness.
#
# Times what the existing benchmark suite does not: how long it
# takes to *check* a Resilient program (parse + 130-pass typechecker
# + optional Z3 audit) and how long a full `cargo check` /
# `cargo build` of the compiler itself takes.
#
# Three knobs to vary:
#
#   1. Source size — small / medium / large `.rz` inputs that grow
#      the attribute-driven pass cost.
#   2. Mode — `rz <prog>` parses + checks + interprets;
#              `rz --audit <prog>` adds the Z3 verifier discharge
#              column (requires the binary built with `--features z3`).
#   3. Compiler build — `cargo check` vs `cargo build` from cold and
#      warm, against the compiler tree itself.
#
# Usage:
#   benchmarks/compile_time/run.sh
#   benchmarks/compile_time/run.sh --skip-cold       # don't `cargo clean` first
#   benchmarks/compile_time/run.sh --skip-z3         # skip --audit rows
#
# Output: a markdown table written to
# benchmarks/compile_time/RESULTS.md (overwrites). Suitable to
# diff in a PR description.
#
# This script is contributor infrastructure, not a CI gate. The
# fib(25) perf-gate's track record of hosted-runner variance shows
# why per-PR median comparison doesn't work for absolute timings.

set -euo pipefail
cd "$(dirname "$0")/../.."

SKIP_COLD=0
SKIP_Z3=0
for arg in "$@"; do
    case "$arg" in
        --skip-cold) SKIP_COLD=1 ;;
        --skip-z3)   SKIP_Z3=1   ;;
        *) echo "unknown flag: $arg" >&2; exit 2 ;;
    esac
done

if ! command -v hyperfine >/dev/null 2>&1; then
    echo "error: hyperfine not on PATH." >&2
    echo "  macOS:  brew install hyperfine" >&2
    echo "  linux:  apt-get install hyperfine" >&2
    exit 2
fi

RZ="resilient/target/release/rz"
RZ_Z3="resilient/target/release/rz-with-z3"
SMALL="benchmarks/compile_time/small.rz"
MEDIUM="benchmarks/compile_time/medium.rz"
LARGE="benchmarks/compile_time/large.rz"

# Ensure the default-features release binary is on disk.
if [[ ! -x "$RZ" ]]; then
    echo "Building release binary (default features)..."
    (cd resilient && cargo build --release --quiet)
fi

# Optional Z3 binary for --audit timings. Built into a separate
# artifact so the default-features build's link doesn't get re-done
# every run.
if [[ $SKIP_Z3 -eq 0 ]] && [[ ! -x "$RZ_Z3" ]]; then
    if cd resilient && cargo build --release --features z3 --quiet 2>/dev/null; then
        cp target/release/rz target/release/rz-with-z3
        cd ..
    else
        cd .. 2>/dev/null || true
        echo "warning: --features z3 build failed (libz3 missing?). Skipping --audit rows." >&2
        SKIP_Z3=1
    fi
fi

OUT=benchmarks/compile_time/RESULTS.md
{
    echo "# Compile-time benchmark results"
    echo
    echo "Hardware: $(uname -sm)"
    echo "Date:     $(date -u +%Y-%m-%dT%H:%M:%SZ)"
    echo "Compiler: \`$(${RZ} --version 2>/dev/null || echo unknown)\`"
    echo
    echo "Hyperfine settings: 2 warmup runs, 5 measured runs per row."
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

# Section 1: program-level typecheck + interpret. Wall-time
# dominated by the 130 typechecker passes for the medium / large
# inputs; small.rz is the noise floor (parser + minimal passes +
# 5 ns of arithmetic).
bench "typecheck + interpret (default features)" \
    --command-name "small.rz"  "$RZ $SMALL" \
    --command-name "medium.rz" "$RZ $MEDIUM" \
    --command-name "large.rz"  "$RZ $LARGE"

# Section 2: with --audit. Adds Z3 discharge attempts on every
# contracted call site. The small.rz delta vs Section 1 isolates
# the Z3 verifier setup cost.
if [[ $SKIP_Z3 -eq 0 ]]; then
    bench "typecheck + interpret + Z3 audit" \
        --command-name "small.rz  --audit" "$RZ_Z3 --audit $SMALL" \
        --command-name "medium.rz --audit" "$RZ_Z3 --audit $MEDIUM" \
        --command-name "large.rz  --audit" "$RZ_Z3 --audit $LARGE"
fi

# Section 3: compiler-tree builds. Cold drops the target dir first;
# warm reuses it. The cold row is the "fresh clone" experience;
# warm is the "edit one fn and recompile" experience.
if [[ $SKIP_COLD -eq 0 ]]; then
    echo "Cleaning target dir for cold-build measurement..."
    rm -rf resilient/target
fi

bench "cargo check (compiler tree)" \
    --command-name "cargo check (cold)"  "cargo check --manifest-path resilient/Cargo.toml" \
    --command-name "cargo check (warm)"  "cargo check --manifest-path resilient/Cargo.toml"

bench "cargo build (compiler tree, release)" \
    --command-name "cargo build --release (warm)" "cargo build --release --manifest-path resilient/Cargo.toml"

echo
echo "Done. Results in $OUT"
