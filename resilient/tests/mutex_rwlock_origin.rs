//! RES-4270: compile-time lock-origin tracking.
//!
//! These cases run through the CLI so they exercise the same extension-pass
//! pipeline used by `--typecheck-strict`, rather than calling the private
//! advisory checker directly.

use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rz")
}

fn scratch_file(source: &str) -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "res_mutex_rwlock_origin_{}_{}.rz",
        std::process::id(),
        id
    ));
    std::fs::write(&path, source).expect("write lock-origin fixture");
    path
}

fn run_strict(source: &str) -> Output {
    let path = scratch_file(source);
    let output = Command::new(bin())
        .args(["--typecheck-strict"])
        .arg(&path)
        .output()
        .expect("spawn rz --typecheck-strict");
    let _ = std::fs::remove_file(path);
    output
}

#[test]
fn scalar_binding_is_rejected_at_typecheck() {
    let output = run_strict("let x = 42;\nlet result = mutex_lock(x);\n");
    assert_eq!(
        output.status.code(),
        Some(1),
        "scalar lock argument should fail: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("error[mutex]"), "unexpected: {stderr}");
    assert!(stderr.contains("mutex_lock"), "unexpected: {stderr}");
}

#[test]
fn cross_kind_lock_use_is_rejected_at_typecheck() {
    let output = run_strict("let m = mutex_new(1);\nlet result = rwlock_read(m);\n");
    assert_eq!(
        output.status.code(),
        Some(1),
        "cross-kind lock use should fail: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("rwlock_read"), "unexpected: {stderr}");
    assert!(stderr.contains("mutex_new"), "unexpected: {stderr}");
}

#[test]
fn unknown_function_parameters_remain_permissive() {
    let output = run_strict("fn consume(int value) { let result = mutex_lock(value); }\n");
    assert_eq!(
        output.status.code(),
        Some(0),
        "parameter-origin lock use should remain permissive: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn reassignment_invalidates_the_previous_lock_origin() {
    let output = run_strict("let m = mutex_new(1);\nm = [1, 2];\nlet result = mutex_lock(m);\n");
    assert_eq!(
        output.status.code(),
        Some(0),
        "reassignment must not leave a stale origin: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn nested_shadowing_is_checked_without_poisoning_the_outer_binding() {
    let output = run_strict(
        "let m = mutex_new(1);\nif true { let m = 42; let inner = mutex_lock(m); }\nlet outer = mutex_lock(m);\n",
    );
    assert_eq!(
        output.status.code(),
        Some(1),
        "shadowed scalar should fail: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("error[mutex]"), "unexpected: {stderr}");
    assert!(
        stderr.contains(":2:"),
        "inner shadowed binding was not diagnosed: {stderr}"
    );
    assert!(
        !stderr.contains(":3:"),
        "outer mutex binding was poisoned by the inner shadow: {stderr}"
    );
}
