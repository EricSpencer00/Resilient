use serde_json::Value;
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
        std::env::temp_dir().join(format!("res_3128_{}_{}_{}.rz", tag, std::process::id(), n));
    std::fs::write(&path, body).expect("write scratch file");
    path
}

#[test]
fn fmt_in_place_rewrites_file_to_canonical_form() {
    let path = tmp_file("fmt_in_place", "fn main(){return 1;}\n");
    let output = Command::new(bin())
        .args(["fmt", "--in-place"])
        .arg(&path)
        .output()
        .expect("spawn rz fmt --in-place");

    assert_eq!(
        output.status.code(),
        Some(0),
        "fmt --in-place should succeed; stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    assert!(
        output.stdout.is_empty(),
        "in-place formatter should not print rewritten source; stdout={}",
        String::from_utf8_lossy(&output.stdout)
    );

    let rewritten = std::fs::read_to_string(&path).expect("read formatted file");
    assert_eq!(rewritten, "fn main() {\n    return 1;\n}\n");

    let _ = std::fs::remove_file(&path);
}

#[test]
fn dump_ast_json_emits_parseable_ast() {
    let path = tmp_file("dump_ast_json", "fn main() { return 1; }\nmain();\n");
    let output = Command::new(bin())
        .arg("--dump-ast-json")
        .arg(&path)
        .output()
        .expect("spawn rz --dump-ast-json");

    let _ = std::fs::remove_file(&path);

    assert_eq!(
        output.status.code(),
        Some(0),
        "--dump-ast-json should succeed; stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );

    let json: Value = serde_json::from_slice(&output.stdout).expect("parse AST JSON");
    assert_eq!(json["type"], "Program", "expected Program root: {json}");
    let program = json["body"].as_array().expect("program body array");
    assert!(
        program.iter().any(|node| node["type"] == "Function"),
        "expected function node in AST JSON: {json}"
    );
    assert!(
        program
            .iter()
            .any(|node| node["type"] == "ExprStmt" && node["expr"]["type"] == "Call"),
        "expected top-level call expression in AST JSON: {json}"
    );
}

#[test]
fn stack_usage_reports_budget_failure() {
    let path = tmp_file(
        "stack_usage",
        "#[stack(bytes=1)]\nfn tiny() {\n    return;\n}\n\ntiny();\n",
    );
    let output = Command::new(bin())
        .args(["stack-usage"])
        .arg(&path)
        .output()
        .expect("spawn rz stack-usage");

    let _ = std::fs::remove_file(&path);

    assert_eq!(
        output.status.code(),
        Some(1),
        "stack-usage should fail when a declared budget is exceeded; stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Function") && stdout.contains("Est. Bytes"));
    assert!(stdout.contains("tiny"));
    assert!(stdout.contains("OVER BUDGET"));
}

#[test]
fn version_verbose_extends_short_banner() {
    let short = Command::new(bin())
        .arg("--version")
        .output()
        .expect("spawn rz --version");
    assert_eq!(short.status.code(), Some(0));
    let short_stdout = String::from_utf8_lossy(&short.stdout);
    assert!(
        short_stdout.starts_with("rz ") && short_stdout.contains("pre-1.0"),
        "unexpected short version output: {short_stdout}"
    );

    let verbose = Command::new(bin())
        .args(["--version", "--verbose"])
        .output()
        .expect("spawn rz --version --verbose");
    assert_eq!(verbose.status.code(), Some(0));
    let verbose_stdout = String::from_utf8_lossy(&verbose.stdout);
    assert!(
        verbose_stdout.starts_with(short_stdout.as_ref()),
        "verbose version output should extend the short banner; short={short_stdout} verbose={verbose_stdout}"
    );
    assert!(
        verbose_stdout.contains("target:") && verbose_stdout.contains("profile:"),
        "verbose version output missing build metadata: {verbose_stdout}"
    );
}
