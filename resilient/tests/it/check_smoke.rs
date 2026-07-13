//! RES-225: smoke tests for `resilient check <file>`.
//!
//! Verifies that `check` parses + type-checks without running:
//! - exits 0 for valid source.
//! - exits 1 for a file with a type error.
//! - exits 2 when no path is given.
//! - respects `--quiet` / `-q` (suppresses all output but keeps exit code).

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rz")
}

fn tmp_dir(tag: &str) -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let p = std::env::temp_dir().join(format!(
        "res_check_smoke_{}_{}_{}",
        tag,
        std::process::id(),
        n
    ));
    std::fs::create_dir_all(&p).expect("mkdir");
    p
}

#[test]
fn check_valid_file_exits_zero() {
    let output = Command::new(bin())
        .args(["check", "examples/hello.rz"])
        .output()
        .expect("spawn resilient check");
    assert_eq!(
        output.status.code(),
        Some(0),
        "expected exit 0 for valid file; stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("ok"),
        "expected 'ok' in output; stdout={stdout}"
    );
}

#[test]
fn check_type_error_exits_one() {
    let dir = tmp_dir("type_err");
    let src_path = dir.join("bad.rz");
    // Assign a string to an int — type mismatch.
    std::fs::write(
        &src_path,
        "fn main(int _d) {\n    let x: Int = \"not an int\";\n    return 0;\n}\nmain(0);\n",
    )
    .unwrap();

    let output = Command::new(bin())
        .arg("check")
        .arg(&src_path)
        .output()
        .expect("spawn resilient check");
    assert_eq!(
        output.status.code(),
        Some(1),
        "expected exit 1 for type error; stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn check_missing_path_exits_two() {
    let output = Command::new(bin())
        .arg("check")
        .output()
        .expect("spawn resilient check");
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("requires a file path"),
        "expected usage hint; got: {stderr}"
    );
}

#[test]
fn check_quiet_suppresses_output() {
    let output = Command::new(bin())
        .args(["check", "--quiet", "examples/hello.rz"])
        .output()
        .expect("spawn resilient check --quiet");
    assert_eq!(
        output.status.code(),
        Some(0),
        "expected exit 0 in quiet mode; stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.is_empty(),
        "expected no stdout in quiet mode; got: {stdout}"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    // seed line is printed by the normal driver, not check — should be absent
    assert!(
        !stderr.contains("ok"),
        "expected no stderr in quiet mode; got: {stderr}"
    );
}

#[test]
fn check_short_quiet_flag() {
    let output = Command::new(bin())
        .args(["check", "-q", "examples/hello.rz"])
        .output()
        .expect("spawn resilient check -q");
    assert_eq!(output.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&output.stdout).is_empty());
}

#[test]
fn check_rejects_nonexhaustive_struct_match() {
    // RES-369: `match` on a struct without a wildcard arm must be a
    // type error.  `examples/match_struct_nonexhaustive.rz` is the
    // golden error-case file for this feature.
    let output = Command::new(bin())
        .args(["check", "examples/match_struct_nonexhaustive.rz"])
        .output()
        .expect("spawn resilient check");
    assert_eq!(
        output.status.code(),
        Some(1),
        "expected exit 1 for non-exhaustive struct match; stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Non-exhaustive match on struct"),
        "expected non-exhaustive diagnostic in stderr; got: {stderr}"
    );
}

#[test]
fn check_accepts_exhaustive_struct_match() {
    // RES-369: `match` with a `Point { .. }` wildcard arm is exhaustive
    // and must type-check cleanly.
    let output = Command::new(bin())
        .args(["check", "examples/match_struct_exhaustive.rz"])
        .output()
        .expect("spawn resilient check");
    assert_eq!(
        output.status.code(),
        Some(0),
        "expected exit 0 for exhaustive struct match; stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
}
