//! RES-4597: stack certification must include every executable AST path.

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rz")
}

fn source_file(tag: &str, source: &str) -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("res_4597_{tag}_{}_{}.rz", std::process::id(), n));
    std::fs::write(&path, source).expect("write stack-contract fixture");
    path
}

fn stack_usage(source: &str, tag: &str) -> String {
    let path = source_file(tag, source);
    let output = Command::new(bin())
        .args(["stack-usage"])
        .arg(&path)
        .output()
        .expect("spawn rz stack-usage");
    let _ = std::fs::remove_file(&path);
    assert!(
        output.status.success(),
        "stack-usage failed: status={} stderr={} stdout={}",
        output.status,
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );
    String::from_utf8(output.stdout).expect("stack-usage output is UTF-8")
}

#[test]
fn match_guards_and_arm_bodies_contribute_to_stack_depth() {
    let output = stack_usage(
        r#"
fn leaf(int x) { return x; }
fn dispatch(int x) {
    return match x {
        0 if leaf(x) >= 0 => leaf(x),
        _ => x,
    };
}
"#,
        "match",
    );

    assert!(
        output.lines().any(|line| {
            line.contains("dispatch") && line.contains("128") && line.contains("leaf")
        }),
        "match calls should produce a two-frame dispatch report, got:\n{output}"
    );
}

#[test]
fn try_handlers_contribute_to_stack_depth() {
    let output = stack_usage(
        r#"
fn leaf(int x) { return x; }
fn recover(int x) {
    try {
        leaf(x);
    } catch Timeout {
        return leaf(x);
    }
}
"#,
        "try",
    );

    assert!(
        output.lines().any(|line| {
            line.contains("recover") && line.contains("128") && line.contains("leaf")
        }),
        "try/catch calls should produce a two-frame recover report, got:\n{output}"
    );
}

#[test]
fn recursive_call_in_match_is_reported_as_unbounded() {
    let output = stack_usage(
        r#"
fn loop(int x) {
    return match x {
        0 => 0,
        _ => loop(x - 1),
    };
}
"#,
        "recursive_match",
    );

    assert!(
        output.lines().any(|line| {
            line.contains("loop") && line.contains("unbounded") && line.contains("recursive")
        }),
        "recursive match call should be reported as unbounded, got:\n{output}"
    );
}
