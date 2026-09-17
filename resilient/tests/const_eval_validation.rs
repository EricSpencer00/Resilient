//! RES-4255: end-to-end validation for const declaration scope and uniqueness.

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
        "res_const_eval_validation_{}_{}.rz",
        std::process::id(),
        id
    ));
    std::fs::write(&path, source).expect("write const source");
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

fn assert_rejected(source: &str, expected: &str, line: &str) {
    let output = run_strict(source);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        output.status.code(),
        Some(1),
        "invalid const program unexpectedly passed:\nstdout={}\nstderr={stderr}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(stderr.contains(expected), "missing {expected:?}: {stderr}");
    assert!(
        stderr.contains(line),
        "diagnostic lost source location {line:?}: {stderr}"
    );
}

#[test]
fn identical_top_level_consts_are_rejected_as_duplicates() {
    assert_rejected(
        "const ANSWER = 42;\nconst ANSWER = 42;\nprintln(ANSWER);\n",
        "duplicate const declaration `ANSWER`",
        ":2:7:",
    );
}

#[test]
fn changed_top_level_const_value_is_rejected_as_conflicting() {
    assert_rejected(
        "const ANSWER = 42;\nconst ANSWER = 43;\nprintln(ANSWER);\n",
        "conflicting const declaration `ANSWER`",
        ":2:7:",
    );
}

#[test]
fn conflicting_top_level_const_annotations_are_reported() {
    assert_rejected(
        "const VALUE: int = 1;\nconst VALUE: string = \"one\";\nprintln(VALUE);\n",
        "conflicting const declaration `VALUE`",
        ":2:7:",
    );
}

#[test]
fn function_scoped_consts_are_rejected_at_the_declaration() {
    assert_rejected(
        "fn compute() -> int {\n    const VALUE = 5;\n    return VALUE;\n}\nprintln(compute());\n",
        "function-scoped `const` declarations are not supported; use `let` instead",
        ":2:11:",
    );
}

#[test]
fn distinct_top_level_consts_still_run() {
    let output = run_strict("const ANSWER = 42;\nconst OTHER = ANSWER + 1;\nprintln(OTHER);\n");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "valid const program failed:\nstdout={stdout}\nstderr={stderr}"
    );
    assert!(
        stdout.contains("43"),
        "valid const program did not run: {stdout}"
    );
}
