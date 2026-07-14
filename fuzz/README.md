# Resilient fuzz targets

Fuzz harness for the Resilient toolchain, built on
[`cargo-fuzz`](https://github.com/rust-fuzz/cargo-fuzz) +
libFuzzer.

## Targets

| Target  | Ticket   | What it fuzzes                                                                              |
| ------- | -------- | ------------------------------------------------------------------------------------------- |
| `parse` | RES-201  | The parser: random bytes → UTF-8 filter → `rz -t`. Fails on panic.                          |
| `lex`   | RES-111  | The lexer: random bytes → UTF-8 filter → `rz --dump-tokens`. Fails on panic.                |
| `jit`   | RES-310  | The Cranelift JIT lowering path: random bytes → UTF-8 filter → `rz --jit`. Fails on panic.  |
| `contracts` | RES-3779 (#3779) | The contract-certificate pipeline: random bytes → UTF-8 filter → `rz --emit-contract-certificate`. Fails on panic, or on a written certificate that isn't well-formed JSON with an in-schema `"verdict"`. |
| `z3_translate` | RES-4039 (C-E6, #3933) | The Z3 SMT translation layer (`verifier_z3.rs`'s `prove_*` entry points): seeded/fuzzed contract source → UTF-8 filter → `rz -t`, requires a `--features z3` `rz` build. Fails on panic. |

Additional targets slot in by adding a file under
`fuzz_targets/` and a `[[bin]]` entry in `fuzz/Cargo.toml`; the
GitHub Actions matrix in `.github/workflows/fuzz.yml` picks
them up via the `target:` key.

## Design note: subprocess, not in-process

The compiler crate now exposes a library target, but these fuzz
targets still exercise the shipped CLI boundary. That keeps fuzz
coverage aligned with the parser, lexer, feature flags, diagnostics,
and panic hooks users reach through `rz` instead of depending on
private parser/lexer internals as an in-process fuzzing API. The
harness shells out to the built binary via `RESILIENT_FUZZ_BIN` and
re-raises subprocess crashes as local panics so libFuzzer records the
input.

This is slower than an in-process fuzzer would be — expect
hundreds to a few thousand iterations per second instead of
millions. Still fast enough to find parser panics in a CI
budget. Moving selected targets in-process would require committing a
small public fuzzing API, such as stable parse/lex entry points with
diagnostic guarantees.

## Running locally

```bash
# One-time setup: nightly Rust + cargo-fuzz binary.
rustup install nightly
cargo install cargo-fuzz --locked

# Build the resilient binary (release for speed).
cargo build --release --manifest-path resilient/Cargo.toml

# Run the parse target for 30 seconds.
RESILIENT_FUZZ_BIN=$PWD/resilient/target/release/rz \
  cargo +nightly fuzz run parse --manifest-path fuzz/Cargo.toml -- \
    -max_total_time=30 \
    -timeout=1

# Or the lex target. Same runner invariants — fails on a
# subprocess panic (SIGABRT) but otherwise passes.
RESILIENT_FUZZ_BIN=$PWD/resilient/target/release/rz \
  cargo +nightly fuzz run lex --manifest-path fuzz/Cargo.toml -- \
    -max_total_time=30 \
    -timeout=1

# Or the contracts target (RES-3779). Works on a stock (non-z3)
# build — verdicts just degrade to "unknown". Fails on a subprocess
# panic, OR on a written certificate whose JSON is malformed or
# whose "verdict" field is outside {pass, fail, unknown}.
RESILIENT_FUZZ_BIN=$PWD/resilient/target/release/rz \
  cargo +nightly fuzz run contracts --manifest-path fuzz/Cargo.toml -- \
    -max_total_time=30 \
    -timeout=1

# RES-310: JIT target. Requires the `rz` binary to be built
# with `--features jit`, AND the fuzz crate's own `jit` feature
# (gates the `[[bin]]` entry). Without `--features jit` on the
# compiler, `--jit` exits with a clean error and the fuzzer
# produces no JIT coverage.
cargo build --release --features jit --manifest-path resilient/Cargo.toml
RESILIENT_FUZZ_BIN=$PWD/resilient/target/release/rz \
  cargo +nightly fuzz run jit --features jit \
    --manifest-path fuzz/Cargo.toml -- \
    -max_total_time=30 \
    -timeout=1

# RES-4039 (C-E6): Z3-translation target. Requires the `rz` binary
# to be built with `--features z3`, AND the fuzz crate's own `z3`
# feature (gates the `[[bin]]`). Without `--features z3` on the
# compiler, `verifier_z3` isn't compiled in — every contract clause
# falls back to the hand-rolled folder, so the fuzzer produces no
# Z3-translation coverage. On macOS with a Homebrew z3 install, set
# the bindgen/link env vars first:
#
#   export Z3_SYS_Z3_HEADER=/opt/homebrew/opt/z3/include/z3.h
#   export BINDGEN_EXTRA_CLANG_ARGS="-I/opt/homebrew/opt/z3/include"
#   export LIBRARY_PATH="/opt/homebrew/opt/z3/lib:${LIBRARY_PATH:-}"
#   export DYLD_FALLBACK_LIBRARY_PATH="/opt/homebrew/opt/z3/lib:${DYLD_FALLBACK_LIBRARY_PATH:-}"
cargo build --release --features z3 --manifest-path resilient/Cargo.toml
RESILIENT_FUZZ_BIN=$PWD/resilient/target/release/rz \
  cargo +nightly fuzz run z3_translate --features z3 \
    --manifest-path fuzz/Cargo.toml -- \
    -max_total_time=30 \
    -timeout=5
```

- `-max_total_time=N` caps the fuzz run at N seconds.
- `-timeout=N` kills any single input that takes longer than
  N seconds; libFuzzer records it as a crash.

## When a crash fires

libFuzzer writes the offending input to `fuzz/artifacts/<target>/`
and prints a `Test unit written to artifacts/<target>/crash-<hash>`
line. Per the ticket's rules, reducing each crash to a unit test +
a parser fix is expected to land in the same PR that reports it:

```bash
# Reproduce locally:
rz -t fuzz/artifacts/parse/crash-<hash>

# Add the input to a Rust unit test under
# `resilient/src/lib.rs` or an integration test, asserting that
# parsing the bytes returns an error vec instead of panicking.
# Then fix the parser site.
```

## CI

`.github/workflows/fuzz.yml` runs on manual dispatch. The
workflow input `duration_seconds` controls the per-target
budget (default 30). Any crash artifact uploads as a workflow
artifact with 30-day retention.

Fuzz is NOT wired into the PR gate — per the ticket, it runs on
demand (and/or on merge to `main` in a future iteration).
