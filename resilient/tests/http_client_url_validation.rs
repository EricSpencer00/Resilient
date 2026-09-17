//! RES-4259: compile-time validation for statically known HTTP URLs.

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
        "res_http_client_url_{}_{}.rz",
        std::process::id(),
        id
    ));
    std::fs::write(&path, source).expect("write HTTP source");
    path
}

fn run_strict(source: &str) -> Output {
    let path = scratch_file(source);
    let output = Command::new(bin())
        .args(["--feature", "std", "--typecheck-strict"])
        .arg(&path)
        .output()
        .expect("spawn rz --typecheck-strict");
    let _ = std::fs::remove_file(path);
    output
}

fn run(source: &str) -> Output {
    let path = scratch_file(source);
    let output = Command::new(bin())
        .arg("--feature")
        .arg("std")
        .arg(&path)
        .output()
        .expect("spawn rz");
    let _ = std::fs::remove_file(path);
    output
}

fn assert_rejected(source: &str, expected: &str) {
    let output = run_strict(source);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        output.status.code(),
        Some(1),
        "invalid HTTP call unexpectedly passed:\nstdout={}\nstderr={stderr}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(stderr.contains(expected), "missing {expected:?}: {stderr}");
    assert!(
        stderr.contains(":2:"),
        "diagnostic lost call-site line: {stderr}"
    );
}

#[test]
fn unsupported_literal_scheme_is_rejected_at_compile_time() {
    assert_rejected(
        "fn main() {\n    let result = http_get(\"https://example.com/api\");\n}\nmain();\n",
        "http_get: invalid URL: http_*: only http:// URLs supported",
    );
}

#[test]
fn empty_literal_host_is_rejected_at_compile_time() {
    assert_rejected(
        "fn main() {\n    let result = http_get(\"http:///api\");\n}\nmain();\n",
        "http_get: invalid URL: http_*: empty host",
    );
}

#[test]
fn invalid_literal_port_is_rejected_at_compile_time() {
    assert_rejected(
        "fn main() {\n    let result = http_get(\"http://example.com:99999/api\");\n}\nmain();\n",
        "http_get: invalid URL: http_*: invalid port: 99999",
    );
}

#[test]
fn wrong_http_get_arity_is_rejected_at_compile_time() {
    assert_rejected(
        "fn main() {\n    let result = http_get();\n}\nmain();\n",
        "http_get expects between 1 and 3 argument(s), got 0",
    );
}

#[test]
fn computed_url_keeps_the_runtime_error_value_contract() {
    let output = run(
        "fn main() {\n    let url = \"https://example.com/api\";\n    let result = http_get(url);\n    println(result);\n}\nmain();\n",
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "computed URL failed before runtime:\nstdout={stdout}\nstderr={stderr}"
    );
    assert!(
        stdout.contains("Err(\"http_*: only http:// URLs supported"),
        "computed URL did not preserve runtime Err value: {stdout}"
    );
    assert!(
        stdout.contains("Program executed successfully"),
        "runtime Err changed process success behavior: {stdout}"
    );
}
