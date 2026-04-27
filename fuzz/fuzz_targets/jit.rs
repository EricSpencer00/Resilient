// RES-310: cargo-fuzz target for the Resilient JIT backend
// (Cranelift lowering path).
//
// Invariant: for any UTF-8 byte sequence, the JIT path must
// either lower the program to native code (and run the produced
// `main`) or surface a clean diagnostic and exit non-zero. It
// MUST NOT panic. A subprocess killed by signal (Rust panic ->
// SIGABRT on Linux, similar on macOS) is re-raised as a local
// panic so libFuzzer records the offending input.
//
// Same subprocess pattern as RES-201's `parse` target and
// RES-111's `lex` target — see `fuzz/README.md` for the design
// rationale (TL;DR: `resilient` is binary-only, no library
// surface to link against). We shell out to `rz --jit <file>`
// which routes through:
//
//     lex (lexer)
//   -> parse (parser)
//   -> typecheck
//   -> jit_backend::run (Cranelift lowering)
//
// Coverage is the JIT lowering pass. Inputs that fail in lex,
// parse, or typecheck still surface JIT panics if any earlier
// pass panics on the same input — those are bugs the lex/parse
// targets also catch, which is fine: defence in depth.
//
// Runner expectations:
// - The `rz` binary MUST be built with `--features jit`.
//   Without the feature, `--jit` exits with a clean error (not
//   a panic), so the harness "passes" trivially on every input
//   and produces zero coverage of the lowering path. CI sets
//   `RESILIENT_FUZZ_BIN` to a `--features jit` release binary.
// - Falls back to `rz` on `PATH` locally; that binary too must
//   have been built with the JIT feature for this target to be
//   meaningful.
//
// Run locally (after `cargo build --release --features jit
// --manifest-path resilient/Cargo.toml`):
//
//   RESILIENT_FUZZ_BIN=$PWD/resilient/target/release/rz \
//     cargo +nightly fuzz run jit --manifest-path fuzz/Cargo.toml -- \
//       -max_total_time=30 \
//       -timeout=1
//
// Note: `[[bin]]` entry in `fuzz/Cargo.toml` is gated on the
// fuzz crate's `jit` feature so the harness only builds when
// the user opts in (mirrors the `--features jit` gating on the
// compiler crate). Use `cargo +nightly fuzz run jit --features
// jit` to build + run.

#![no_main]

use libfuzzer_sys::fuzz_target;
use std::io::Write;
use std::process::Command;

fuzz_target!(|data: &[u8]| {
    // Reject non-UTF-8 up front — the lexer takes `&str` and
    // libFuzzer would otherwise burn budget on inputs the
    // pipeline rejects before reaching the JIT.
    let Ok(src) = std::str::from_utf8(data) else { return };

    let mut f = tempfile::Builder::new()
        .prefix("res_fuzz_jit_")
        .suffix(".rs")
        .tempfile()
        .expect("could not create tempfile");
    f.write_all(src.as_bytes()).expect("write to tempfile");
    f.flush().expect("flush tempfile");

    let bin = std::env::var("RESILIENT_FUZZ_BIN")
        .unwrap_or_else(|_| "rz".to_string());
    let status = Command::new(&bin)
        .arg("--jit")
        // RES-174: --jit invokes typecheck implicitly only when
        // the user passes other flags; pass `--seed 0` so any
        // randomised codepath is deterministic across runs (matches
        // the `parse` target's invocation).
        .arg("--seed")
        .arg("0")
        .arg(f.path())
        .output();

    let Ok(output) = status else {
        // Couldn't spawn the binary — not a JIT bug. libFuzzer
        // skips this input.
        return;
    };

    // A subprocess killed by a signal (Rust panic -> SIGABRT)
    // returns `status.code() == None`. Re-raise as a local panic
    // so libFuzzer records the offending input in its crash
    // report. Non-zero exit codes are FINE — JIT-unsupported
    // constructs, type errors, parse errors all exit non-zero
    // cleanly and are not bugs.
    if output.status.code().is_none() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        panic!(
            "rz --jit process crashed (signal) on fuzz input:\n\
             stderr tail: {}",
            stderr.lines().rev().take(5).collect::<Vec<_>>().join(" | ")
        );
    }
});
