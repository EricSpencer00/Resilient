//! RES-199: integration-level round-trip smoke tests for `resilient fmt`.
//!
//! The property-based tests (proptest, 1000 cases, shrinking enabled) live
//! in `src/formatter.rs` under `#[cfg(all(test, feature = "proptest"))]`
//! and run with:
//!
//!   cargo test --features proptest
//!
//! This file covers the CLI wiring: we write a few canonical programs to
//! temp files, invoke `resilient fmt`, and assert the output is byte-for-byte
//! equal to the input (i.e., `format(parse(src)) == src` for already-canonical
//! source).

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_resilient")
}

fn tmp_file(tag: &str, body: &str) -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let path =
        std::env::temp_dir().join(format!("res_199_{}_{}_{}.res", tag, std::process::id(), n));
    std::fs::write(&path, body).expect("write scratch file");
    path
}

fn fmt_file(path: &PathBuf) -> (String, String, i32) {
    let out = Command::new(bin())
        .arg("fmt")
        .arg(path)
        .output()
        .expect("failed to spawn resilient binary");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code().unwrap_or(-1),
    )
}

/// A simple function declaration in canonical form round-trips unchanged.
#[test]
fn canonical_fn_decl_roundtrips() {
    let src = "\
fn add(int a, int b) -> int {
    return a + b;
}
";
    let path = tmp_file("fn_decl", src);
    let (stdout, stderr, code) = fmt_file(&path);
    assert_eq!(code, 0, "fmt exited non-zero.\nstderr: {}", stderr);
    assert_eq!(
        stdout, src,
        "fmt output differs from canonical input.\nexpected:\n{}\ngot:\n{}",
        src, stdout
    );
}

/// A let binding at top level round-trips unchanged.
#[test]
fn canonical_let_roundtrips() {
    let src = "let x = 42;\n";
    let path = tmp_file("let", src);
    let (stdout, stderr, code) = fmt_file(&path);
    assert_eq!(code, 0, "fmt exited non-zero.\nstderr: {}", stderr);
    assert_eq!(
        stdout, src,
        "fmt output differs from canonical input.\nexpected:\n{}\ngot:\n{}",
        src, stdout
    );
}

/// A function with if/else in canonical form round-trips unchanged.
#[test]
fn canonical_if_else_roundtrips() {
    let src = "\
fn sign(int n) -> int {
    if n > 0 {
        return 1;
    } else {
        return 0;
    }
}
";
    let path = tmp_file("if_else", src);
    let (stdout, stderr, code) = fmt_file(&path);
    assert_eq!(code, 0, "fmt exited non-zero.\nstderr: {}", stderr);
    assert_eq!(
        stdout, src,
        "fmt output differs from canonical input.\nexpected:\n{}\ngot:\n{}",
        src, stdout
    );
}

/// A while loop in canonical form round-trips unchanged.
#[test]
fn canonical_while_roundtrips() {
    let src = "\
fn count(int n) {
    let i = 0;
    while i < n {
        i = i + 1;
    }
}
";
    let path = tmp_file("while", src);
    let (stdout, stderr, code) = fmt_file(&path);
    assert_eq!(code, 0, "fmt exited non-zero.\nstderr: {}", stderr);
    assert_eq!(
        stdout, src,
        "fmt output differs from canonical input.\nexpected:\n{}\ngot:\n{}",
        src, stdout
    );
}

/// A struct declaration in canonical form round-trips unchanged.
#[test]
fn canonical_struct_roundtrips() {
    let src = "\
struct Point {
    int x,
    int y,
}
";
    let path = tmp_file("struct", src);
    let (stdout, stderr, code) = fmt_file(&path);
    assert_eq!(code, 0, "fmt exited non-zero.\nstderr: {}", stderr);
    assert_eq!(
        stdout, src,
        "fmt output differs from canonical input.\nexpected:\n{}\ngot:\n{}",
        src, stdout
    );
}

/// Two top-level items separated by a blank line round-trip unchanged.
#[test]
fn canonical_multi_item_roundtrips() {
    let src = "\
fn one() {
    return 1;
}

fn two() {
    return 2;
}
";
    let path = tmp_file("multi", src);
    let (stdout, stderr, code) = fmt_file(&path);
    assert_eq!(code, 0, "fmt exited non-zero.\nstderr: {}", stderr);
    assert_eq!(
        stdout, src,
        "fmt output differs from canonical input.\nexpected:\n{}\ngot:\n{}",
        src, stdout
    );
}

/// fmt returns exit code 1 for a file with parse errors.
#[test]
fn fmt_exits_nonzero_on_parse_error() {
    let src = "fn broken { this is not valid }\n";
    let path = tmp_file("broken", src);
    let (_stdout, _stderr, code) = fmt_file(&path);
    assert_ne!(code, 0, "expected non-zero exit for broken source");
}
