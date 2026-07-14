#!/usr/bin/env bash
# RES-3987 (D-E1): run the on-device `.rzbc` loader binary under
# `qemu-system-arm`'s `lm3s6965evb` machine and verify both (a) the
# semihosting output matches the fixture's expected result and (b)
# QEMU's own process exit status is success. Mirrors
# `docs/EMBEDDED_PIPELINE.md` section 4's Cortex-M plan: the loader
# binary reports pass/fail via `cortex-m-semihosting`'s
# `hprintln!`/`debug::exit()`, and `debug::exit(EXIT_SUCCESS)`
# translates to QEMU exiting 0 — no serial-port scraping needed.
#
# Prerequisite: `scripts/build_loader_demo.sh` has already produced a
# release ELF for `thumbv7em-none-eabihf` (the `embedded-runtime.yml`
# CI job runs that script immediately before this one).
#
# Usage:
#   resilient-runtime-loader-demo/run_qemu.sh
#
# Exits 0 if QEMU reports success and the expected output is present,
# non-zero otherwise (wrong result, QEMU error exit, or timeout).

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"

# RES-1584: the repo-root `.cargo/config.toml` pins `target-dir` to
# `resilient/target` for every workspace member, including this crate
# (which declares its own empty `[workspace]` table but still inherits
# the ancestor config's `target-dir` key during cargo's config
# discovery walk) — so the ELF lands under `resilient/target/`, not
# under `resilient-runtime-loader-demo/target/`.
ELF="$ROOT/resilient/target/thumbv7em-none-eabihf/release/resilient-runtime-loader-demo"

if [ ! -f "$ELF" ]; then
  echo "error: loader-demo ELF not found at $ELF" >&2
  echo "  run scripts/build_loader_demo.sh first" >&2
  exit 1
fi

EXPECTED="loader ok: Int(21)"
TIMEOUT_SECS=30
OUTPUT_FILE="$(mktemp)"
trap 'rm -f "$OUTPUT_FILE"' EXIT

set +e
timeout "${TIMEOUT_SECS}s" qemu-system-arm \
  -M lm3s6965evb \
  -cpu cortex-m4 \
  -nographic \
  -semihosting-config enable=on,target=native \
  -kernel "$ELF" \
  > "$OUTPUT_FILE" 2>&1
QEMU_EXIT=$?
set -e

echo "--- QEMU output (exit=$QEMU_EXIT) ---"
cat "$OUTPUT_FILE"
echo "--- end QEMU output ---"

if [ "$QEMU_EXIT" -eq 124 ]; then
  echo "error: qemu-system-arm timed out after ${TIMEOUT_SECS}s" >&2
  exit 1
fi

if [ "$QEMU_EXIT" -ne 0 ]; then
  echo "error: qemu-system-arm exited $QEMU_EXIT (expected 0 — debug::exit(EXIT_SUCCESS))" >&2
  exit 1
fi

if ! grep -qF "$EXPECTED" "$OUTPUT_FILE"; then
  echo "error: expected semihosting output '$EXPECTED' not found" >&2
  exit 1
fi

echo "ok: loader-demo ran under QEMU and reported '$EXPECTED'"
