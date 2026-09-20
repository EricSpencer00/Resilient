//! Regression coverage for transitive `#[no_panic]` certification.

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

fn typecheck(source: &str) -> (bool, String) {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    let path: PathBuf = std::env::temp_dir().join(format!(
        "res_no_panic_transitive_{}_{}.rz",
        std::process::id(),
        id
    ));
    std::fs::write(&path, source).expect("write no-panic fixture");
    let output = Command::new(env!("CARGO_BIN_EXE_rz"))
        .arg("check")
        .arg(&path)
        .output()
        .expect("spawn rz check");
    let _ = std::fs::remove_file(path);
    let diagnostics = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    (output.status.success(), diagnostics)
}

#[test]
fn no_panic_rejects_a_direct_unwrap() {
    let (success, diagnostics) = typecheck(
        r#"
#[no_panic]
fn direct() -> int {
    return unwrap(Err(1));
}

direct();
"#,
    );
    assert!(!success, "direct unwrap unexpectedly passed: {diagnostics}");
    assert!(
        diagnostics.contains("direct") && diagnostics.contains("unwrap"),
        "diagnostic should identify the certified function and trigger: {diagnostics}"
    );
}

#[test]
fn no_panic_rejects_an_unwrap_in_a_reachable_helper() {
    let (success, diagnostics) = typecheck(
        r#"
#[no_panic]
fn certified() -> int {
    return helper();
}

fn helper() -> int {
    return unwrap(Err(1));
}

certified();
"#,
    );
    assert!(
        !success,
        "transitive unwrap unexpectedly passed: {diagnostics}"
    );
    assert!(
        diagnostics.contains("certified")
            && diagnostics.contains("helper")
            && diagnostics.contains("unwrap"),
        "diagnostic should identify the path to the trigger: {diagnostics}"
    );
}

#[test]
fn no_panic_rejects_a_panic_through_a_named_function_value() {
    let (success, diagnostics) = typecheck(
        r#"
#[no_panic]
fn certified() -> int {
    let callback = helper;
    return callback();
}

fn helper() -> int {
    return unwrap(Err(1));
}

certified();
"#,
    );
    assert!(
        !success,
        "first-class function call unexpectedly passed: {diagnostics}"
    );
    assert!(
        diagnostics.contains("certified")
            && diagnostics.contains("helper")
            && diagnostics.contains("unwrap"),
        "diagnostic should identify the first-class callee and trigger: {diagnostics}"
    );
}

#[test]
fn no_panic_rejects_a_panic_through_a_closure_returning_function_value() {
    let (success, diagnostics) = typecheck(
        r#"
fn make_callback() -> fn() -> int {
    let callback = helper;
    return fn() -> int { return callback(); };
}

#[no_panic]
fn certified() -> int {
    let callback = make_callback();
    return callback();
}

fn helper() -> int {
    return unwrap(Err(1));
}

certified();
"#,
    );
    assert!(
        !success,
        "closure function-value call unexpectedly passed: {diagnostics}"
    );
    assert!(
        diagnostics.contains("certified")
            && diagnostics.contains("helper")
            && diagnostics.contains("unwrap"),
        "diagnostic should identify the closure's captured callee and trigger: {diagnostics}"
    );
}
