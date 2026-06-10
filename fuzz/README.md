# Resilient fuzz targets

Fuzz harness for the Resilient toolchain, built on
[`cargo-fuzz`](https://github.com/rust-fuzz/cargo-fuzz) +
libFuzzer.

## Targets

| Target  | Ticket   | What it fuzzes                                                                              |
| ------- | -------- | ------------------------------------------------------------------------------------------- |
| `parse` | RES-201  | The parser: random bytes → UTF-8 filter → `resilient -t`. Fails on panic.                   |
| `lex`   | RES-111  | The lexer: random bytes → UTF-8 filter → `resilient --dump-tokens`. Fails on panic.         |
| `jit`   | RES-310  | The Cranelift JIT lowering path: random bytes → UTF-8 filter → `rz --jit`. Fails on panic.  |

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
