//! RES-4545: function-type boundaries must preserve variance soundness.
//!
//! A callback's input type is contravariant and its return type is
//! covariant. The checker must not use the ordinary bidirectional `any`
//! compatibility rule for either position.

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rz")
}

fn check_src(src: &str) -> (String, i32) {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let path: PathBuf = std::env::temp_dir().join(format!(
        "res_4545_function_variance_{}_{}.rz",
        std::process::id(),
        n
    ));
    std::fs::write(&path, src).expect("write temporary source");
    let output = Command::new(bin())
        .args(["check", path.to_str().expect("temporary path is UTF-8")])
        .output()
        .expect("spawn rz check");
    let _ = std::fs::remove_file(&path);
    let diagnostics = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    (diagnostics, output.status.code().unwrap_or(-1))
}

#[test]
fn narrower_callback_is_rejected_before_runtime() {
    let (diagnostics, code) = check_src(
        r#"
fn only_int(int x) -> int {
    return x % 2;
}

fn invoke(fn(any) -> int f) -> int {
    return f("not an int");
}

invoke(only_int);
"#,
    );
    assert_ne!(
        code, 0,
        "narrow callback must fail static checking: {diagnostics}"
    );
    assert!(
        diagnostics.contains("Type mismatch") || diagnostics.contains("type mismatch"),
        "expected a callback type mismatch: {diagnostics}"
    );
}

#[test]
fn wider_callback_remains_usable_for_narrower_calls() {
    let (diagnostics, code) = check_src(
        r#"
fn accepts_any(any x) -> int {
    return 1;
}

fn invoke(fn(int) -> int f) -> int {
    return f(7);
}

invoke(accepts_any);
"#,
    );
    assert_eq!(code, 0, "wider callback should remain valid: {diagnostics}");
}

#[test]
fn callback_with_dynamic_return_is_not_promoted_to_int() {
    let (diagnostics, code) = check_src(
        r#"
fn returns_any(int x) -> any {
    return "not an int";
}

fn invoke(fn(int) -> int f) -> int {
    return f(7);
}

invoke(returns_any);
"#,
    );
    assert_ne!(
        code, 0,
        "dynamic return must not satisfy int callback: {diagnostics}"
    );
    assert!(
        diagnostics.contains("Type mismatch") || diagnostics.contains("type mismatch"),
        "expected a callback return mismatch: {diagnostics}"
    );
}
