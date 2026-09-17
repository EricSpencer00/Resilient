//! RES-4257: end-to-end derive target and call-site enforcement.
//!
//! These cases use the strict CLI so they exercise parser attribute
//! collection, typechecking, the derive extension pass, and runtime
//! dispatch together.

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
        "res_derive_use_sites_{}_{}.rz",
        std::process::id(),
        id
    ));
    std::fs::write(&path, source).expect("write derive fixture");
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

const POINT: &str = "struct Point { int x, int y }\n";

#[test]
fn missing_partial_ord_is_rejected_at_the_operator() {
    let output = run_strict(&format!(
        "{POINT}\nfn main() {{\n    let p = new Point {{ x: 1, y: 2 }};\n    let q = new Point {{ x: 1, y: 3 }};\n    println(p < q);\n}}\nmain();\n"
    ));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        output.status.code(),
        Some(1),
        "missing PartialOrd unexpectedly passed: {stderr}"
    );
    assert!(stderr.contains("struct `Point`"), "unexpected: {stderr}");
    assert!(stderr.contains("PartialOrd"), "unexpected: {stderr}");
    assert!(
        stderr.contains(":6:"),
        "diagnostic lost operator line: {stderr}"
    );
}

#[test]
fn partial_ord_derived_structs_still_run() {
    let output = run_strict(&format!(
        "#[derive(PartialOrd)]\n{POINT}\nfn main() {{\n    let p = new Point {{ x: 1, y: 2 }};\n    let q = new Point {{ x: 1, y: 3 }};\n    println(p < q);\n}}\nmain();\n"
    ));
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        output.status.code(),
        Some(0),
        "valid PartialOrd program failed: {stderr}"
    );
    assert!(stdout.contains("true"), "unexpected output: {stdout}");
    assert!(
        stdout.contains("Program executed successfully"),
        "program did not run: {stdout}"
    );
}

#[test]
fn missing_partial_eq_is_rejected_for_struct_equality() {
    let output = run_strict(&format!(
        "{POINT}\nfn main() {{\n    let p = new Point {{ x: 1, y: 2 }};\n    let q = new Point {{ x: 1, y: 3 }};\n    println(p == q);\n}}\nmain();\n"
    ));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        output.status.code(),
        Some(1),
        "missing PartialEq unexpectedly passed: {stderr}"
    );
    assert!(stderr.contains("PartialEq"), "unexpected: {stderr}");
    assert!(stderr.contains("Eq"), "unexpected: {stderr}");
}

#[test]
fn derive_on_a_non_struct_is_rejected_at_the_attribute() {
    let output = run_strict("#[derive(Debug)]\nfn main() { println(1); }\nmain();\n");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        output.status.code(),
        Some(1),
        "derive on a function unexpectedly passed: {stderr}"
    );
    assert!(
        stderr.contains("target `main` is not a declared struct"),
        "unexpected: {stderr}"
    );
    assert!(
        stderr.contains(":1:1:"),
        "missing attribute location: {stderr}"
    );
}

#[test]
fn parameter_struct_types_are_checked_inside_function_bodies() {
    let output = run_strict(&format!(
        "{POINT}\nfn less(Point left, Point right) -> bool {{ return left < right; }}\nfn main() {{\n    let p = new Point {{ x: 1, y: 2 }};\n    let q = new Point {{ x: 1, y: 3 }};\n    println(less(p, q));\n}}\nmain();\n"
    ));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        output.status.code(),
        Some(1),
        "parameter comparison unexpectedly passed: {stderr}"
    );
    assert!(stderr.contains("PartialOrd"), "unexpected: {stderr}");
    assert!(
        stderr.contains(":3:"),
        "missing parameter comparison line: {stderr}"
    );
}
