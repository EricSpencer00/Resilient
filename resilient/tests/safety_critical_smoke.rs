//! RES-778: integration tests for `--safety-critical`.
//!
//! The new mode should promote `assume(false)` / L0006 from a warning
//! to a hard error, while preserving today's default lint behavior.

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rz")
}

fn tmp_file(tag: &str, body: &str) -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let path =
        std::env::temp_dir().join(format!("res_778_{}_{}_{}.rz", tag, std::process::id(), n));
    std::fs::write(&path, body).expect("write scratch");
    path
}

#[test]
fn lint_keeps_l0006_as_warning_by_default() {
    let src = tmp_file("lint_warn", "fn f() {\n    assume(false);\n}\n");
    let out = Command::new(bin())
        .args(["lint"])
        .arg(&src)
        .output()
        .expect("spawn rz lint");
    assert_eq!(
        out.status.code(),
        Some(1),
        "default lint mode should keep L0006 as a warning; stdout={} stderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("warning[L0006]"),
        "expected warning-severity L0006, got:\n{stdout}"
    );
    let _ = std::fs::remove_file(&src);
}

#[test]
fn lint_promotes_l0006_under_safety_critical() {
    let src = tmp_file("lint_error", "fn f() {\n    assume(false);\n}\n");
    let out = Command::new(bin())
        .args(["lint", "--safety-critical"])
        .arg(&src)
        .output()
        .expect("spawn rz lint --safety-critical");
    assert_eq!(
        out.status.code(),
        Some(2),
        "safety-critical lint mode should escalate L0006; stdout={} stderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("error[L0006]"),
        "expected error-severity L0006, got:\n{stdout}"
    );
    let _ = std::fs::remove_file(&src);
}

#[test]
fn check_rejects_assume_false_under_safety_critical() {
    let src = tmp_file("check_error", "fn f() {\n    assume(false);\n}\n");
    let out = Command::new(bin())
        .args(["check", "--safety-critical"])
        .arg(&src)
        .output()
        .expect("spawn rz check --safety-critical");
    assert_eq!(
        out.status.code(),
        Some(1),
        "safety-critical check mode should fail compilation; stdout={} stderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("error[L0006]"),
        "expected L0006 hard error, got:\n{stderr}"
    );
    assert!(
        stderr.contains("assume(false)"),
        "diagnostic should name assume(false), got:\n{stderr}"
    );
    let _ = std::fs::remove_file(&src);
}

#[test]
fn safety_critical_mode_ignores_allow_comment_for_l0006() {
    let src = tmp_file(
        "allow_ignored",
        "fn f() {\n    // resilient: allow L0006\n    assume(false);\n}\n",
    );
    let out = Command::new(bin())
        .args(["check", "--safety-critical"])
        .arg(&src)
        .output()
        .expect("spawn rz check --safety-critical");
    assert_eq!(
        out.status.code(),
        Some(1),
        "allow comment must not bypass safety-critical L0006; stdout={} stderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("error[L0006]"),
        "expected unsuppressible L0006 error, got:\n{stderr}"
    );
    let _ = std::fs::remove_file(&src);
}

#[test]
fn help_lists_safety_critical_flag() {
    let out = Command::new(bin())
        .arg("--help")
        .output()
        .expect("spawn rz --help");
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("--safety-critical"),
        "--help should mention --safety-critical; got:\n{stdout}"
    );
}
