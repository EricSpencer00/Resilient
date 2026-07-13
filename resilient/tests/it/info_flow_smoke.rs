//! RES-2824: end-to-end smoke tests for the information-flow /
//! non-interference pass. Unlike the in-module unit tests (which inject
//! `#[secret]` / `#[public]` / `#[declassify]` straight into the shared
//! attribute registry), these drive the *full* pipeline through the
//! compiled `rz` binary: real attribute syntax → `cfg_attr` parser →
//! `feature_attrs` registry → `info_flow::check`.

use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rz")
}

fn run_check(example: &str) -> (String, String, Option<i32>) {
    let output = Command::new(bin())
        .arg("check")
        .arg(format!("examples/{example}"))
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("failed to spawn rz");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn leak_example_is_rejected() {
    // `publish` (#[public]) returns the value of `read_secret`
    // (#[secret]) directly — an explicit flow that must be rejected.
    let (stdout, stderr, code) = run_check("info_flow_leak.rz");
    assert_eq!(
        code,
        Some(1),
        "leak example must fail `rz check`; stdout={stdout} stderr={stderr}"
    );
    let combined = format!("{stdout}{stderr}");
    assert!(
        combined.contains("info-flow"),
        "expected an info-flow diagnostic; got:\n{combined}"
    );
    assert!(
        combined.contains("publish") && combined.contains("read_secret"),
        "diagnostic should name the public sink and the secret source; got:\n{combined}"
    );
}

#[test]
fn declassify_example_is_accepted() {
    // Routing the same secret through the #[declassify] `redact` launders
    // it, so the public sink type-checks.
    let (stdout, stderr, code) = run_check("info_flow_declassify.rz");
    assert_eq!(
        code,
        Some(0),
        "declassified flow must pass `rz check`; stdout={stdout} stderr={stderr}"
    );
}

#[test]
fn declassify_example_runs_and_launders() {
    let output = Command::new(bin())
        .arg("examples/info_flow_declassify.rz")
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("failed to spawn rz");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        output.status.code(),
        Some(0),
        "declassify example must run; stdout={stdout} stderr={stderr}"
    );
    // publish(5)  -> redact(38) -> 1
    // publish(-1) -> redact(-4) -> 0
    assert!(
        stdout.contains('1') && stdout.contains('0'),
        "expected laundered outputs 1 and 0; got:\n{stdout}"
    );
}
