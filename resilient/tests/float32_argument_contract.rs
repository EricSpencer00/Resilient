//! Integration coverage for RES-4258's compile-time float conversion checks.

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
        "res_float32_argument_contract_{}_{}.rz",
        std::process::id(),
        id
    ));
    std::fs::write(&path, source).expect("write float32 contract fixture");
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
fn string_argument_is_rejected_before_execution() {
    let output = run_strict("let value = \"not a number\";\nlet result = as_f32(value);\n");
    assert_eq!(output.status.code(), Some(1), "stderr: {:?}", output.stderr);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("error: as_f32 expects an int or float argument, got String"),
        "unexpected diagnostic: {stderr}"
    );
}

#[test]
fn bool_argument_is_rejected_before_execution() {
    let output = run_strict("let result = as_f64(true);\n");
    assert_eq!(output.status.code(), Some(1), "stderr: {:?}", output.stderr);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("error: as_f64 expects an int or float argument, got Bool"),
        "unexpected diagnostic: {stderr}"
    );
}

#[test]
fn struct_and_array_arguments_are_rejected_before_execution() {
    let struct_output = run_strict(
        "struct Sample { int value, }\nlet sample = new Sample { value: 1 };\nlet result = as_f32(sample);\n",
    );
    assert_eq!(
        struct_output.status.code(),
        Some(1),
        "stderr: {:?}",
        struct_output.stderr
    );
    assert!(
        String::from_utf8_lossy(&struct_output.stderr)
            .contains("as_f32 expects an int or float argument, got Sample")
    );

    let array_output = run_strict("let values = [1, 2, 3];\nlet result = as_f32(values);\n");
    assert_eq!(
        array_output.status.code(),
        Some(1),
        "stderr: {:?}",
        array_output.stderr
    );
    assert!(
        String::from_utf8_lossy(&array_output.stderr)
            .contains("as_f32 expects an int or float argument, got Array")
    );
}

#[test]
fn unresolved_array_element_remains_permissive() {
    let output = run_strict(
        "let values = [identity(1)];\nlet value = values[0];\nlet result = as_f32(value);\nprintln(result);\n",
    );
    assert_eq!(
        output.status.code(),
        Some(0),
        "unresolved array element should stay permissive: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn numeric_arguments_still_typecheck() {
    let output = run_strict(
        "let integer = 42;\nlet single = as_f32(integer);\nlet double = as_f64(single);\nprintln(double);\n",
    );
    assert_eq!(
        output.status.code(),
        Some(0),
        "numeric conversions should remain valid: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
}
