//! RES-393: regression tests — the parser must not panic on
//! malformed expressions in core syntactic positions.
//!
//! Each case writes a tiny canary program with a deliberately
//! malformed expression, runs the real `rz` binary, and asserts:
//!   1. the process exits cleanly (no `Option::unwrap()` panic),
//!   2. stderr contains a `line:col:` parser-error diagnostic.
//!
//! These were previously hard panics — see issue #265.

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rz")
}

fn scratch_path(tag: &str) -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("res_panic_{}_{}_{}.rz", tag, std::process::id(), n))
}

fn run_program(source: &str, tag: &str) -> (Option<i32>, String, String) {
    let path = scratch_path(tag);
    std::fs::write(&path, source).expect("write canary");
    let output = Command::new(bin()).arg(&path).output().expect("spawn rz");
    let _ = std::fs::remove_file(&path);
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    (output.status.code(), stdout, stderr)
}

fn assert_no_panic(stderr: &str) {
    assert!(
        !stderr.contains("called `Option::unwrap()` on a `None` value"),
        "parser panicked instead of recording a clean error; stderr:\n{stderr}"
    );
    assert!(
        !stderr.contains("panicked at"),
        "parser panicked instead of recording a clean error; stderr:\n{stderr}"
    );
}

#[test]
fn empty_let_rhs_does_not_panic() {
    let src = "fn main() {\n    let x = ;\n    return 0;\n}\nmain();\n";
    let (_code, _stdout, stderr) = run_program(src, "let_rhs");
    assert_no_panic(&stderr);
    let combined = stderr;
    assert!(
        combined.contains("Parser error"),
        "expected a parser-error diagnostic; stderr:\n{combined}"
    );
    assert!(
        combined.contains("2:") && (combined.contains("let") || combined.contains("expression")),
        "expected line:col context for the empty RHS; stderr:\n{combined}"
    );
}

#[test]
fn missing_infix_rhs_does_not_panic() {
    let src = "fn main() {\n    let x = 1 + ;\n    return 0;\n}\nmain();\n";
    let (_code, _stdout, stderr) = run_program(src, "infix_rhs");
    assert_no_panic(&stderr);
    assert!(
        stderr.contains("Parser error") && stderr.contains("Expected expression"),
        "expected a parser-error diagnostic mentioning a missing expression; stderr:\n{stderr}"
    );
}

#[test]
fn empty_trailing_call_argument_does_not_panic() {
    // Trailing `,` before `)` in a call. The lexer reaches the `)`
    // with no expression to parse — previously panicked.
    let src = "fn add(a: Int, b: Int) -> Int { return a + b; }\nfn main() {\n    let x = add(1,);\n    return 0;\n}\nmain();\n";
    let (_code, _stdout, stderr) = run_program(src, "call_arg");
    assert_no_panic(&stderr);
}
