//! RES-4454: `rz fmt` must preserve expression grouping.

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
        "res_formatter_precedence_{}_{}.rz",
        std::process::id(),
        id
    ));
    std::fs::write(&path, source).expect("write formatter fixture");
    path
}

fn run_fmt(path: &PathBuf, args: &[&str]) -> Output {
    Command::new(bin())
        .arg("fmt")
        .args(args)
        .arg(path)
        .output()
        .expect("spawn rz fmt")
}

#[test]
fn formatting_preserves_grouping_and_runtime_result() {
    let path = scratch_file(
        "fn calculate(int _d) -> int { return (10 - (3 - 1)); }\nprintln(calculate(0));\n",
    );

    let formatted = run_fmt(&path, &["--in-place"]);
    assert_eq!(
        formatted.status.code(),
        Some(0),
        "formatter failed: stderr={}",
        String::from_utf8_lossy(&formatted.stderr)
    );

    let source = std::fs::read_to_string(&path).expect("read formatted source");
    assert!(
        source.contains("10 - (3 - 1)"),
        "formatter dropped right-hand grouping:\n{source}"
    );

    let executed = Command::new(bin())
        .arg(&path)
        .output()
        .expect("run formatted source");
    assert!(
        executed.status.success(),
        "formatted program failed: stdout={} stderr={}",
        String::from_utf8_lossy(&executed.stdout),
        String::from_utf8_lossy(&executed.stderr)
    );
    assert!(
        String::from_utf8_lossy(&executed.stdout).contains("8"),
        "formatted program changed the result: stdout={} stderr={}",
        String::from_utf8_lossy(&executed.stdout),
        String::from_utf8_lossy(&executed.stderr)
    );

    let _ = std::fs::remove_file(path);
}

#[test]
fn formatting_parenthesizes_prefix_and_postfix_targets() {
    let path = scratch_file(
        "let neg = -(1 + 2);\nlet called = (f + g)(x);\nlet field = (x + y).value;\nlet indexed = (x + y)[0];\nlet tried = (x + y)?;\n",
    );

    let formatted = run_fmt(&path, &[]);
    assert_eq!(
        formatted.status.code(),
        Some(0),
        "formatter failed: stderr={}",
        String::from_utf8_lossy(&formatted.stderr)
    );
    let source = String::from_utf8_lossy(&formatted.stdout);
    assert!(
        source.contains("-(1 + 2)"),
        "prefix grouping lost:\n{source}"
    );
    assert!(
        source.contains("(f + g)(x)"),
        "call target grouping lost:\n{source}"
    );
    assert!(
        source.contains("(x + y).value"),
        "field target grouping lost:\n{source}"
    );
    assert!(
        source.contains("(x + y)[0]"),
        "index target grouping lost:\n{source}"
    );
    assert!(
        source.contains("(x + y)?"),
        "try operand grouping lost:\\n{source}"
    );

    let _ = std::fs::remove_file(path);
}

#[test]
fn precedence_safe_output_is_idempotent() {
    let path =
        scratch_file("let a = (1 + 2) * 3;\nlet b = 10 - (3 - 1);\nlet c = !(true || false);\n");

    let first = run_fmt(&path, &["--in-place"]);
    assert_eq!(
        first.status.code(),
        Some(0),
        "first format failed: stderr={}",
        String::from_utf8_lossy(&first.stderr)
    );
    let once = std::fs::read_to_string(&path).expect("read first format");

    let second = run_fmt(&path, &[]);
    assert_eq!(
        second.status.code(),
        Some(0),
        "second format failed: stderr={}",
        String::from_utf8_lossy(&second.stderr)
    );
    assert_eq!(
        once,
        String::from_utf8_lossy(&second.stdout),
        "precedence-safe formatting must be idempotent"
    );

    let _ = std::fs::remove_file(path);
}
