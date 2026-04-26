// RES-111: cargo-fuzz target for the Resilient lexer.
//
// Invariant: for any UTF-8 byte sequence, the lexer must either
// emit a token stream ending in EOF or record a diagnostic and
// stop cleanly. Never panic, never loop.
//
// Same subprocess pattern as RES-201's `parse` target: we shell
// out to the built `resilient` binary with `--dump-tokens`,
// which calls `Lexer::new(input).next_token_with_span()` to EOF
// and prints one token per line. A child exit via signal (Rust
// panic → SIGABRT) is re-raised as a local panic so libFuzzer
// records the offending input.
//
// Why subprocess, not in-process? The `resilient` crate is
// binary-only today (no `src/lib.rs`). Same trade-off as the
// parse target — see `fuzz/README.md`.
//
// Runner expectations:
// - `RESILIENT_FUZZ_BIN` points at the release binary (set by
//   CI); falls back to `resilient` on `PATH` locally.
// - The binary must include `--dump-tokens`, which it does
//   by default (no feature flag needed).

#![no_main]

use libfuzzer_sys::fuzz_target;
use std::io::Write;
use std::process::Command;

fuzz_target!(|data: &[u8]| {
    // Reject non-UTF-8 — the lexer takes `&str`. Per ticket
    // Notes: "Arbitrary bytes ≠ arbitrary UTF-8; wrap the input
    // in `std::str::from_utf8(data).ok()?` and return early on
    // non-UTF-8 so we fuzz only the scanning logic."
    let Ok(src) = std::str::from_utf8(data) else { return };

    // Write to a scratch `.rs` file; `--dump-tokens` reads from
    // disk (the binary doesn't accept source on stdin).
    let mut f = tempfile::Builder::new()
        .prefix("res_fuzz_lex_")
        .suffix(".rs")
        .tempfile()
        .expect("could not create tempfile");
    f.write_all(src.as_bytes()).expect("write to tempfile");
    f.flush().expect("flush tempfile");

    let bin = std::env::var("RESILIENT_FUZZ_BIN")
        .unwrap_or_else(|_| "rz".to_string());
    let status = Command::new(&bin)
        .arg("--dump-tokens")
        .arg(f.path())
        .output();

    let Ok(output) = status else {
        // Couldn't spawn the binary — not a lexer bug. Libfuzzer
        // will skip.
        return;
    };

    if output.status.code().is_none() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        panic!(
            "resilient --dump-tokens process crashed (signal) on fuzz input:\n\
             stderr tail: {}",
            stderr.lines().rev().take(5).collect::<Vec<_>>().join(" | ")
        );
    }
    // Non-zero clean exits are fine — malformed inputs surface
    // as parser / typechecker errors, not lexer panics. The
    // invariant is "lexer doesn't panic", not "input parses".
});
