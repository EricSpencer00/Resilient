//! RES-198: integration tests for `resilient lint <file>`.
//!
//! Each test writes a tiny program to a temp file, shells out to
//! the real binary, and asserts on the exit code + stdout/stderr.
//! The unit tests in `src/lint.rs` cover the detection logic;
//! these pin the CLI wiring: arg parsing, exit codes, `--deny` /
//! `--allow` escalation, unknown-code handling.

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_resilient")
}

fn tmp_file(tag: &str, body: &str) -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let path =
        std::env::temp_dir().join(format!("res_198_{}_{}_{}.rs", tag, std::process::id(), n));
    std::fs::write(&path, body).expect("write scratch");
    path
}

#[test]
fn lint_exits_zero_on_clean_program() {
    let src = tmp_file(
        "clean",
        "fn f(int a) requires a > 0 {\n    let used = a + 1;\n    return used;\n}\n",
    );
    let out = Command::new(bin())
        .args(["lint"])
        .arg(&src)
        .output()
        .expect("spawn lint");
    assert!(
        out.status.success(),
        "expected 0, got {:?}\nstdout: {}\nstderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    let _ = std::fs::remove_file(&src);
}

#[test]
fn lint_exits_one_on_warning() {
    let src = tmp_file(
        "warn",
        "fn f(int a) {\n    let unused = 42;\n    return a;\n}\n",
    );
    let out = Command::new(bin())
        .args(["lint"])
        .arg(&src)
        .output()
        .expect("spawn lint");
    assert_eq!(
        out.status.code(),
        Some(1),
        "expected 1 (warning), got {:?}",
        out.status
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("L0001"), "stdout: {stdout}");
    assert!(stdout.contains("warning"), "stdout: {stdout}");
    let _ = std::fs::remove_file(&src);
}

#[test]
fn lint_deny_escalates_to_error_exit_two() {
    let src = tmp_file(
        "deny",
        "fn f(int a) {\n    let unused = 42;\n    return a;\n}\n",
    );
    let out = Command::new(bin())
        .args(["lint"])
        .arg(&src)
        .args(["--deny", "L0001"])
        .output()
        .expect("spawn lint");
    assert_eq!(
        out.status.code(),
        Some(2),
        "expected 2 (error), got {:?}",
        out.status
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("error[L0001]"), "stdout: {stdout}");
    let _ = std::fs::remove_file(&src);
}

#[test]
fn lint_allow_flag_suppresses_code() {
    let src = tmp_file(
        "allow",
        "fn f(int a) requires a > 0 {\n    let unused = 42;\n    return a;\n}\n",
    );
    let out = Command::new(bin())
        .args(["lint"])
        .arg(&src)
        .args(["--allow", "L0001"])
        .output()
        .expect("spawn lint");
    assert!(
        out.status.success(),
        "expected 0 under --allow L0001, got {:?}\nstdout: {}",
        out.status,
        String::from_utf8_lossy(&out.stdout),
    );
    let _ = std::fs::remove_file(&src);
}

#[test]
fn lint_rejects_unknown_deny_code() {
    let src = tmp_file("unknown_deny", "fn f() { return 0; }\n");
    let out = Command::new(bin())
        .args(["lint"])
        .arg(&src)
        .args(["--deny", "LX999"])
        .output()
        .expect("spawn lint");
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("unknown lint code"), "stderr: {stderr}");
    let _ = std::fs::remove_file(&src);
}

#[test]
fn lint_requires_file_argument() {
    let out = Command::new(bin())
        .args(["lint"])
        .output()
        .expect("spawn lint");
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("<file>") || stderr.contains("file path"),
        "stderr: {stderr}"
    );
}

#[test]
fn lint_errors_on_missing_file() {
    let out = Command::new(bin())
        .args(["lint", "/nonexistent/path.rs"])
        .output()
        .expect("spawn lint");
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn lint_prints_location_in_rust_like_format() {
    let src = tmp_file(
        "fmt",
        "fn f(int a) {\n    let unused = 42;\n    return a;\n}\n",
    );
    let out = Command::new(bin())
        .args(["lint"])
        .arg(&src)
        .output()
        .expect("spawn lint");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let src_path = src.to_string_lossy().to_string();
    // Expected: `<path>:<line>:<col>: warning[L0001]: ...`
    let prefix = format!("{}:2:", src_path);
    assert!(
        stdout.contains(&prefix),
        "expected `{prefix}...` in stdout, got: {stdout}"
    );
    let _ = std::fs::remove_file(&src);
}

#[test]
fn lint_handles_multiple_codes_per_invocation() {
    // A program triggering L0001 + L0003 + L0005. --deny L0003
    // should fail with error exit 2; L0001 + L0005 stay as
    // warnings (so even without their own --deny, the overall
    // exit is driven by the error).
    let src = tmp_file(
        "multi",
        "fn f(int x) {\n    let unused = 42;\n    if x == x { return 1; }\n    return;\n}\n",
    );
    let out = Command::new(bin())
        .args(["lint"])
        .arg(&src)
        .args(["--deny", "L0003"])
        .output()
        .expect("spawn lint");
    assert_eq!(out.status.code(), Some(2));
    let stdout = String::from_utf8_lossy(&out.stdout);
    // L0003 escalated to error.
    assert!(stdout.contains("error[L0003]"), "stdout: {stdout}");
    // L0001 + L0005 still warnings.
    assert!(stdout.contains("warning[L0001]"), "stdout: {stdout}");
    assert!(stdout.contains("warning[L0005]"), "stdout: {stdout}");
    let _ = std::fs::remove_file(&src);
}
