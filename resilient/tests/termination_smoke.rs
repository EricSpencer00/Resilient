//! RES-398: integration tests for `--strict-termination`.
//!
//! Each test shells out to the real binary, asserting that:
//!   - without the flag, recursive programs run normally;
//!   - with the flag, annotated recursive fns pass;
//!   - with the flag, unannotated direct recursion is rejected
//!     at compile time (exit code 1) with a useful message.

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
        std::env::temp_dir().join(format!("res_398_{}_{}_{}.rz", tag, std::process::id(), n));
    std::fs::write(&path, body).expect("write scratch");
    path
}

#[test]
fn unannotated_recursion_runs_without_flag() {
    // Default mode: termination check is off; program runs.
    let src = tmp_file(
        "default_off",
        "fn loops(int n) { if n <= 0 { return; } loops(n - 1); }\nloops(2);\n",
    );
    let out = Command::new(bin()).arg(&src).output().expect("spawn rz");
    assert!(
        out.status.success(),
        "expected success without --strict-termination, got {:?}\nstderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr),
    );
    let _ = std::fs::remove_file(&src);
}

#[test]
fn unannotated_recursion_rejected_under_strict_flag() {
    let src = tmp_file(
        "strict_missing",
        "fn loops(int n) { if n <= 0 { return; } loops(n - 1); }\nloops(2);\n",
    );
    let out = Command::new(bin())
        .args(["--strict-termination"])
        .arg(&src)
        .output()
        .expect("spawn rz");
    assert!(
        !out.status.success(),
        "expected failure under --strict-termination, got {:?}",
        out.status
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("recursive but has no termination annotation"),
        "stderr should explain the missing annotation, got: {stderr}"
    );
    assert!(
        stderr.contains("@decreases") && stderr.contains("@may_diverge"),
        "stderr should suggest both annotation forms, got: {stderr}"
    );
    let _ = std::fs::remove_file(&src);
}

#[test]
fn decreases_annotation_satisfies_strict_flag() {
    let src = tmp_file(
        "strict_decreases",
        "// @decreases n\nfn loops(int n) { if n <= 0 { return; } loops(n - 1); }\nloops(2);\n",
    );
    let out = Command::new(bin())
        .args(["--strict-termination"])
        .arg(&src)
        .output()
        .expect("spawn rz");
    assert!(
        out.status.success(),
        "expected success with `// @decreases n`, got {:?}\nstderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr),
    );
    let _ = std::fs::remove_file(&src);
}

#[test]
fn may_diverge_annotation_satisfies_strict_flag() {
    let src = tmp_file(
        "strict_may_diverge",
        "// @may_diverge\nfn loops(int n) { if n <= 0 { return; } loops(n - 1); }\nloops(2);\n",
    );
    let out = Command::new(bin())
        .args(["--strict-termination"])
        .arg(&src)
        .output()
        .expect("spawn rz");
    assert!(
        out.status.success(),
        "expected success with `// @may_diverge`, got {:?}\nstderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr),
    );
    let _ = std::fs::remove_file(&src);
}

#[test]
fn non_recursive_fn_unaffected_by_strict_flag() {
    // A non-recursive fn needs no annotation, regardless of the flag.
    let src = tmp_file(
        "strict_nonrec",
        "fn double(int x) { return x + x; }\nlet y = double(7);\nprintln(\"y=\" + y);\n",
    );
    let out = Command::new(bin())
        .args(["--strict-termination"])
        .arg(&src)
        .output()
        .expect("spawn rz");
    assert!(
        out.status.success(),
        "non-recursive fn should pass under --strict-termination, got {:?}",
        out.status
    );
    let _ = std::fs::remove_file(&src);
}

#[test]
fn unannotated_mutual_recursion_rejected_under_strict_flag() {
    // RES-774/RES-784: mutual recursion (SCC size > 1) is also rejected without annotations
    let src = tmp_file(
        "strict_mutual_rec",
        "fn f(int n) { if n > 0 { g(n - 1); } }\nfn g(int n) { if n > 0 { f(n + 1); } }\nf(2);\n",
    );
    let out = Command::new(bin())
        .args(["--strict-termination"])
        .arg(&src)
        .output()
        .expect("spawn rz");
    assert!(
        !out.status.success(),
        "expected failure for unannotated mutual recursion under --strict-termination, got {:?}",
        out.status
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("mutually recursive") || stderr.contains("recursive"),
        "stderr should explain the mutual recursion, got: {stderr}"
    );
    let _ = std::fs::remove_file(&src);
}

#[test]
fn annotated_mutual_recursion_accepted_under_strict_flag() {
    // With @decreases annotations, mutual recursion is accepted
    let src = tmp_file(
        "strict_mutual_annotated",
        "// @decreases n\nfn f(int n) { if n > 0 { g(n - 1); } }\n// @decreases n\nfn g(int n) { if n > 0 { f(n - 1); } }\nf(2);\n",
    );
    let out = Command::new(bin())
        .args(["--strict-termination"])
        .arg(&src)
        .output()
        .expect("spawn rz");
    assert!(
        out.status.success(),
        "expected success for annotated mutual recursion under --strict-termination, got {:?}\nstderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr),
    );
    let _ = std::fs::remove_file(&src);
}
