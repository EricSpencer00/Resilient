// RES-3779: cargo-fuzz target for the contract-certificate pipeline
// (RES-3859 / #3854 Tier 3, `--emit-contract-certificate`).
//
// Invariant: for any UTF-8 byte sequence, routing it through
// `rz <file> --emit-contract-certificate <out>` must:
//   1. Never crash the process (no panic / signal).
//   2. If a certificate file is produced, it must be well-formed
//      JSON whose schema is `resilient-contract-certificate/v1`,
//      whose `schema_version` is the current
//      `contract_certificate::SCHEMA_VERSION` (C-E5, #3933), and
//      whose every per-clause `"verdict"` is one of `"pass"`,
//      `"fail"`, or `"unknown"` — the fixed set defined by
//      `contract_verify::Verdict` and serialized by
//      `contract_certificate::emit` (see
//      `resilient/src/contract_certificate.rs`).
//
// Certificate emission is independent of the z3 feature — without
// it every verdict degrades honestly to `"unknown"` (see the
// `non_z3_build_reports_unknown_golden` test in
// `contract_certificate.rs`), so this target requires no special
// build flags and needs no z3 toolchain to be meaningful.
//
// Same CLI-boundary pattern as RES-201's `parse` target and
// RES-111's `lex` target — see `fuzz/README.md` for the subprocess
// design rationale. We shell out to `rz` instead of depending on
// `contract_certificate::emit` as a private in-process fuzzing API.
//
// Runner expectations:
// - `RESILIENT_FUZZ_BIN` points at the built `rz` binary (CI sets
//   this to the release build); falls back to `rz` on `PATH`
//   locally.
// - No feature flags required on the `rz` build — the certificate
//   path works on a stock (non-z3) build.

#![no_main]

use libfuzzer_sys::fuzz_target;
use std::io::Write;
use std::process::Command;

/// The only verdict strings `contract_certificate::emit` may ever
/// write. Any other value in the `"verdict"` field is a schema
/// violation — see `Verdict::{Pass,Fail,Unknown}` in
/// `resilient/src/contract_verify.rs`.
const VALID_VERDICTS: &[&str] = &["pass", "fail", "unknown"];

/// Walk a `serde_json::Value` looking for every `"verdict"` key at
/// any depth and assert its value is one of `VALID_VERDICTS`. The
/// certificate schema nests verdicts under `functions[].clauses[]`,
/// but walking unconditionally is robust to shape drift — the
/// invariant we care about (no verdict outside the fixed set) holds
/// regardless of where in the document it appears.
fn assert_verdicts_valid(value: &serde_json::Value, src: &str) {
    match value {
        serde_json::Value::Object(map) => {
            if let Some(serde_json::Value::String(v)) = map.get("verdict") {
                assert!(
                    VALID_VERDICTS.contains(&v.as_str()),
                    "contract certificate emitted an invalid verdict {:?} for input:\n{}",
                    v,
                    src
                );
            }
            for v in map.values() {
                assert_verdicts_valid(v, src);
            }
        }
        serde_json::Value::Array(items) => {
            for v in items {
                assert_verdicts_valid(v, src);
            }
        }
        _ => {}
    }
}

fuzz_target!(|data: &[u8]| {
    // Reject non-UTF-8 up front — the lexer/parser take `&str`.
    let Ok(src) = std::str::from_utf8(data) else { return };

    let mut src_file = tempfile::Builder::new()
        .prefix("res_fuzz_contracts_")
        .suffix(".rs")
        .tempfile()
        .expect("could not create source tempfile");
    src_file.write_all(src.as_bytes()).expect("write to tempfile");
    src_file.flush().expect("flush tempfile");

    // Reserve a path for the certificate output. `NamedTempFile`
    // creates the file up front so the path is guaranteed unique;
    // `rz` overwrites it in place via `std::fs::write`.
    let cert_file = tempfile::Builder::new()
        .prefix("res_fuzz_contracts_cert_")
        .suffix(".json")
        .tempfile()
        .expect("could not create cert tempfile");
    let cert_path = cert_file.path().to_path_buf();

    let bin = std::env::var("RESILIENT_FUZZ_BIN").unwrap_or_else(|_| "rz".to_string());
    let status = Command::new(&bin)
        .arg(src_file.path())
        .arg("--emit-contract-certificate")
        .arg(&cert_path)
        .output();

    let Ok(output) = status else {
        // Couldn't spawn the binary — not a bug in the certificate
        // path. libFuzzer will effectively skip this input.
        return;
    };

    // A subprocess killed by a signal (Rust panic -> SIGABRT on
    // Linux, similar on macOS) returns `status.code() == None`.
    // Re-raise as a local panic so libFuzzer records the offending
    // input in its crash report.
    if output.status.code().is_none() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        panic!(
            "rz --emit-contract-certificate process crashed (signal) on fuzz input:\n\
             stderr tail: {}",
            stderr.lines().rev().take(5).collect::<Vec<_>>().join(" | ")
        );
    }
    // Non-zero exit codes ARE fine — parse errors, type errors,
    // import errors, etc. all exit non-zero without ever reaching
    // certificate emission. The invariant is "no panic", plus
    // "any certificate that IS written is schema-valid".

    let Ok(contents) = std::fs::read_to_string(&cert_path) else {
        return;
    };
    if contents.is_empty() {
        // `rz` didn't reach the emit step for this input (e.g. a
        // parse error returned before the certificate write) — the
        // reserved tempfile is still empty. Nothing to validate.
        return;
    }

    let json: serde_json::Value = serde_json::from_str(&contents).unwrap_or_else(|e| {
        panic!(
            "--emit-contract-certificate wrote non-JSON output: {e}\n\
             contents:\n{contents}\n\
             source input:\n{src}"
        )
    });

    assert_eq!(
        json.get("schema").and_then(|v| v.as_str()),
        Some("resilient-contract-certificate/v1"),
        "unexpected certificate schema for input:\n{src}\ndocument:\n{contents}"
    );
    // C-E5: schema_version must always be present and equal to the
    // one current version this fuzz target knows about — bump this
    // alongside `contract_certificate::SCHEMA_VERSION` if it changes.
    assert_eq!(
        json.get("schema_version")
            .and_then(serde_json::Value::as_u64),
        Some(1),
        "unexpected certificate schema_version for input:\n{src}\ndocument:\n{contents}"
    );

    assert_verdicts_valid(&json, src);
});
