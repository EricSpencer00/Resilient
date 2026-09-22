//! RES-4545: function-type boundaries must preserve variance soundness.
//!
//! Callback inputs are contravariant and callback returns are covariant. These
//! tests exercise the CLI boundary so the issue reproducer is checked before
//! interpretation rather than only through an internal relation helper.

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

fn check_source(source: &str) -> (String, i32) {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    let path: PathBuf = std::env::temp_dir().join(format!(
        "res_4545_function_type_variance_{}_{}.rz",
        std::process::id(),
        id
    ));
    std::fs::write(&path, source).expect("write temporary Resilient source");
    let output = Command::new(env!("CARGO_BIN_EXE_rz"))
        .args(["--typecheck-strict"])
        .arg(&path)
        .output()
        .expect("spawn rz --typecheck-strict");
    let _ = std::fs::remove_file(path);
    let diagnostics = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    (diagnostics, output.status.code().unwrap_or(-1))
}

#[test]
fn issue_reproducer_is_rejected_before_interpretation() {
    let (diagnostics, code) = check_source(
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
        "a narrow callback must fail static checking: {diagnostics}"
    );
    assert!(
        diagnostics.contains("Type mismatch") || diagnostics.contains("type mismatch"),
        "expected a callback type mismatch: {diagnostics}"
    );
}

#[test]
fn callback_accepting_any_remains_valid_for_int_calls() {
    let (diagnostics, code) = check_source(
        r#"
fn accepts_any(any value) -> int {
    return 1;
}

fn invoke(fn(int) -> int f) -> int {
    return f(7);
}

invoke(accepts_any);
"#,
    );
    assert_eq!(
        code, 0,
        "a callback accepting any should remain valid: {diagnostics}"
    );
}

#[test]
fn callback_returns_int_when_any_is_expected() {
    let (diagnostics, code) = check_source(
        r#"
fn returns_int(int value) -> int {
    return value;
}

fn invoke(fn(int) -> any f) -> any {
    return f(7);
}

invoke(returns_int);
"#,
    );
    assert_eq!(
        code, 0,
        "a covariant concrete return should remain valid: {diagnostics}"
    );
}

#[test]
fn callback_returning_any_is_not_promoted_to_int() {
    let (diagnostics, code) = check_source(
        r#"
fn returns_any(int value) -> any {
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
        "an unconstrained callback return must not satisfy int: {diagnostics}"
    );
    assert!(
        diagnostics.contains("Type mismatch") || diagnostics.contains("type mismatch"),
        "expected a callback return mismatch: {diagnostics}"
    );
}

#[test]
fn generic_and_composite_callback_signatures_remain_valid() {
    let (diagnostics, code) = check_source(
        r#"
fn apply<T>(T value, fn(T) -> T callback) -> T {
    return callback(value);
}

fn double(int value) -> int {
    return value * 2;
}

fn invoke(fn((int, string)) -> (int, string) callback) -> (int, string) {
    return callback((7, "ok"));
}

let result = apply(5, double);
let pair = invoke(fn((int, string) value) -> (int, string) {
    return value;
});
"#,
    );
    assert_eq!(
        code, 0,
        "generic/composite callback signatures should remain valid: {diagnostics}"
    );
}

#[test]
fn generic_callback_can_precede_binding_argument() {
    let (diagnostics, code) = check_source(
        r#"
fn apply<T>(fn(T) -> T callback, T value) -> T {
    return callback(value);
}

fn double(int value) -> int {
    return value * 2;
}

let result = apply(double, 5);
"#,
    );
    assert_eq!(
        code, 0,
        "a callback may precede the argument that binds its generic type: {diagnostics}"
    );
}
