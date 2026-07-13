//! RES-2825: end-to-end smoke tests for the self-composition
//! non-interference verifier. Gated on `--features z3` — without the
//! feature the pass is an advisory no-op, so these only run under
//! `cargo test --features z3`.

#![cfg(feature = "z3")]

use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rz")
}

fn check(example: &str) -> (String, String, Option<i32>) {
    let out = Command::new(bin())
        .arg("check")
        .arg(format!("examples/{example}"))
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("spawn rz");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code(),
    )
}

#[test]
fn proven_noninterferent_passes() {
    let (stdout, stderr, code) = check("noninterference_ok.rz");
    assert_eq!(
        code,
        Some(0),
        "non-interferent fn should pass `rz check`; stdout={stdout} stderr={stderr}"
    );
    assert!(
        format!("{stdout}{stderr}").contains("non-interferent"),
        "expected a proof line naming the result non-interferent; got:\n{stdout}\n{stderr}"
    );
}

#[test]
fn leak_is_rejected_with_counterexample() {
    let (stdout, stderr, code) = check("noninterference_leak.rz");
    assert_eq!(
        code,
        Some(1),
        "a leaking fn must be rejected by `rz check`; stdout={stdout} stderr={stderr}"
    );
    let combined = format!("{stdout}{stderr}");
    assert!(
        combined.contains("noninterference") && combined.contains("leaks"),
        "expected a non-interference leak diagnostic; got:\n{combined}"
    );
    assert!(
        combined.contains("counterexample"),
        "expected a counterexample in the diagnostic; got:\n{combined}"
    );
}
