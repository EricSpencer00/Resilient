// RES-4039: cargo-fuzz target for the Z3 SMT translation layer
// (`resilient/src/verifier_z3.rs`'s `prove_*` entry points — LIA,
// BV32, overflow-safe BV64, alias-disjointness, and non-interference
// self-composition), reached through the typechecker's
// `requires`/`ensures`/`#[noninterference(...)]`/region-disjointness
// verification passes.
//
// Invariant: for any UTF-8 byte sequence, running the full pipeline
// (lex -> parse -> typecheck, which drives the clause -> Z3 AST
// translation and solve step for every contract obligation the
// hand-rolled folder can't decide) must never panic. Parse errors,
// type errors, "could not prove" diagnostics, and Z3 timeouts are
// all fine — those are `Result`/diagnostic paths, not panics.
//
// Why subprocess, not in-process against `prove_auto` /
// `prove_with_axioms_and_timeout` / `prove_alias_disjoint` /
// `prove_noninterference` directly? Those functions take `&Node`,
// and both `Node` and the `verifier_z3` module are crate-private (no
// `pub`) — there is no committed in-process fuzzing API for the
// translation layer, same as the parser/lexer (see fuzz/README.md's
// "Design note: subprocess, not in-process"). Exposing `Node`,
// `Parser`, and `verifier_z3` as public API is a real surface
// decision the README already flags as a separate undertaking, out
// of scope for a fuzz-coverage PR. Instead this target shells out to
// `rz`, requiring a `--features z3` build: every fuzzed/seeded
// program that contains a contract obligation routes through the
// exact same translation entry points a real user's `rz check`
// invocation would use.
//
// Runner expectations:
// - `RESILIENT_FUZZ_BIN` MUST point at an `rz` binary built with
//   `--features z3` (CI does this — see
//   `.github/workflows/fuzz.yml`). Without it, `verifier_z3` isn't
//   compiled in at all: every contract clause falls back to the
//   hand-rolled folder only (RES-060..065), so this target "passes"
//   trivially and exercises zero Z3 translation code — same caveat
//   as the `jit` target without `--features jit`.
// - Falls back to `rz` on `PATH` locally; that binary too must be a
//   z3-featured build for this target to be meaningful.
// - Typecheck runs by default (`--no-typecheck` opts out), so no
//   extra flag is needed to reach the verifier passes — `-t` is
//   passed anyway for parity with the `parse` target and to make the
//   invocation self-documenting.
//
// Run locally (after building a z3-featured release binary — see
// fuzz/README.md's "Local z3 build env vars" block for the macOS
// `bindgen`/link vars):
//
//   cargo build --release --features z3 --manifest-path resilient/Cargo.toml
//   RESILIENT_FUZZ_BIN=$PWD/resilient/target/release/rz \
//     cargo +nightly fuzz run z3_translate --features z3 \
//       --manifest-path fuzz/Cargo.toml -- \
//       -max_total_time=30 \
//       -timeout=5
//
// A longer per-input timeout (5s) than parse/lex/jit/contracts (1s):
// a Z3 tautology / BV32 / self-composition query can legitimately
// take longer than a lex/parse pass before the verifier's own
// internal `timeout_ms` budget elapses and it returns `None` /
// timed-out — 1s was tuned for the non-solver targets, not this one.
//
// The seed corpus (`fuzz/seed-corpus/z3_translate/`) biases mutation
// toward constructs that actually reach the Z3 path: bitwise
// `requires`/`ensures` clauses (BV32 theory selection), region
// `&mut[R]` disjointness obligations, and `#[noninterference(...)]`
// self-composition (both a provable-OK case and a provable-leak
// case) — random bytes alone essentially never produce a
// well-typed function with a non-trivial contract, so without seeds
// libFuzzer would spend its whole budget short-circuiting in
// lex/parse and never touch `verifier_z3` at all.

#![no_main]

use libfuzzer_sys::fuzz_target;
use std::io::Write;
use std::process::Command;

fuzz_target!(|data: &[u8]| {
    // Reject non-UTF-8 up front — the lexer/parser take `&str`.
    let Ok(src) = std::str::from_utf8(data) else { return };

    let mut f = tempfile::Builder::new()
        .prefix("res_fuzz_z3_translate_")
        .suffix(".rs")
        .tempfile()
        .expect("could not create tempfile");
    f.write_all(src.as_bytes()).expect("write to tempfile");
    f.flush().expect("flush tempfile");

    let bin = std::env::var("RESILIENT_FUZZ_BIN").unwrap_or_else(|_| "rz".to_string());
    let status = Command::new(&bin)
        .arg("-t")
        .arg("--seed")
        .arg("0")
        .arg(f.path())
        .output();

    let Ok(output) = status else {
        // Couldn't spawn the binary — not a verifier bug. libFuzzer
        // will effectively skip this input.
        return;
    };

    // A subprocess killed by a signal (Rust panic -> SIGABRT on
    // Linux, similar on macOS) returns `status.code() == None`.
    // Re-raise as a local panic so libFuzzer records the offending
    // input in its crash report.
    if output.status.code().is_none() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        panic!(
            "rz -t (z3-featured) process crashed (signal) on fuzz input:\n\
             stderr tail: {}",
            stderr.lines().rev().take(5).collect::<Vec<_>>().join(" | ")
        );
    }
    // Non-zero exit codes ARE fine — parse errors, type errors,
    // unproven contracts, and Z3 timeouts all exit non-zero cleanly.
    // The invariant is "no panic", not "no diagnostic".
});
