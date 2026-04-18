// RES-201: cargo-fuzz target for the Resilient parser.
//
// Invariant: for any byte sequence, the parser must produce a
// `(Node, Vec<String>)` (errors-as-data) rather than panic. The
// existing `Parser::record_error` path makes this achievable —
// this target pins the contract.
//
// Shape of the target:
// 1. Filter the raw bytes to UTF-8 (the parser takes `&str`). A
//    random byte sequence is usually not valid UTF-8; reject
//    early so libFuzzer doesn't waste budget on the str ctor.
// 2. Write the input to a temp file.
// 3. Spawn `resilient --typecheck --seed 0 <tempfile>` and
//    wait for it to exit. A clean exit (any status) is a pass;
//    a signal (SIGABRT from a Rust panic) is a fail we re-raise
//    as a panic so libFuzzer records the input.
//
// Why subprocess, not in-process? The `resilient` crate is
// binary-only (no `src/lib.rs`), so there's no library we can
// link against and call `parse(&str)` directly. Subprocess is
// ~1ms per iteration — slower than in-process would be, but
// still millions of iters/hour, which is enough to surface
// parser panics in a CI budget.
//
// Runner expectations:
// - The `resilient` binary must be built (debug or release).
// - Its path is read from `RESILIENT_FUZZ_BIN` (preferred) or
//   assumed to be `resilient` on `PATH`. The workflow in
//   `.github/workflows/fuzz.yml` sets `RESILIENT_FUZZ_BIN` to
//   the release build.

#![no_main]

use libfuzzer_sys::fuzz_target;
use std::io::Write;
use std::process::Command;

fuzz_target!(|data: &[u8]| {
    // Step 1: reject non-UTF-8 up front. The parser takes
    // `&str`, and the lexer's `read_identifier` etc. assume
    // the input already decoded cleanly.
    let Ok(src) = std::str::from_utf8(data) else { return };

    // Step 2: write to a scratch file. `tempfile` handles
    // cleanup on drop even if the fuzzer aborts mid-run.
    let mut f = tempfile::Builder::new()
        .prefix("res_fuzz_")
        .suffix(".rs")
        .tempfile()
        .expect("could not create tempfile");
    f.write_all(src.as_bytes()).expect("write to tempfile");
    f.flush().expect("flush tempfile");

    // Step 3: spawn the resilient binary. Prefer
    // `RESILIENT_FUZZ_BIN` (CI sets this), fall back to PATH.
    let bin = std::env::var("RESILIENT_FUZZ_BIN")
        .unwrap_or_else(|_| "resilient".to_string());
    let status = Command::new(&bin)
        .arg("-t")
        .arg("--seed")
        .arg("0")
        .arg(f.path())
        .output();

    let Ok(output) = status else {
        // Couldn't spawn the binary — not a parser bug. Libfuzzer
        // will effectively skip; returning early prevents this
        // from being recorded as an input that "caused" a failure.
        return;
    };

    // A subprocess killed by a signal (Rust panic → SIGABRT on
    // Linux, or similar on macOS) returns `status.code() ==
    // None`. Re-raise as a local panic so libFuzzer records the
    // offending input in its crash report.
    if output.status.code().is_none() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        panic!(
            "resilient process crashed (signal) on fuzz input:\n\
             stderr tail: {}",
            stderr.lines().rev().take(5).collect::<Vec<_>>().join(" | ")
        );
    }
    // Non-zero exit codes ARE fine — parse errors, type errors,
    // runtime errors on `--typecheck` all exit non-zero. The
    // invariant we're checking is "no panic", not "no error".
});
