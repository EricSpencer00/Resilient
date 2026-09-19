//! RES-4115: E0023 coverage for lexer integer-literal overflow.
//!
//! These subprocess tests exercise the shipped CLI so the assertion covers
//! the actual lexer-to-stderr path while keeping the legacy output contract
//! separate from the opt-in stable-code form.

use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rz")
}

fn source_file() -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "res4115_integer_literal_overflow_{}_{}.rz",
        std::process::id(),
        n
    ));
    std::fs::write(
        &path,
        "let decimal = 9223372036854775808;\nlet hexadecimal = 0xffffffffffffffff;\n",
    )
    .expect("write overflow source");
    path
}

fn run_check(rich: bool) -> (Output, PathBuf) {
    let path = source_file();
    let mut command = Command::new(bin());
    command.arg("check").arg(&path);
    if rich {
        command.env("RESILIENT_RICH_DIAG", "1");
    } else {
        command.env_remove("RESILIENT_RICH_DIAG");
    }
    let output = command.output().expect("run overflow check");
    (output, path)
}

#[test]
fn overflow_keeps_legacy_text_without_rich_codes() {
    let (output, path) = run_check(false);
    let _ = std::fs::remove_file(path);

    assert!(
        output.status.success(),
        "overflow fallback should preserve check success; stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("integer literal `9223372036854775808` overflows i64"),
        "decimal overflow diagnostic missing: {stderr}"
    );
    assert!(
        stderr.contains("integer literal `0xffffffffffffffff` overflows i64"),
        "radix overflow diagnostic missing: {stderr}"
    );
    assert!(
        !stderr.contains("E0023"),
        "stable code leaked into legacy output: {stderr}"
    );
}

#[test]
fn overflow_adds_e0023_in_rich_mode() {
    let (output, path) = run_check(true);
    let _ = std::fs::remove_file(path);

    assert!(
        output.status.success(),
        "rich overflow check should preserve check success; stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(stderr.matches("[E0023]").count(), 2, "stderr={stderr}");
}

#[test]
fn explain_lists_and_renders_e0023() {
    let explain = Command::new(bin())
        .args(["explain", "E0023"])
        .output()
        .expect("run rz explain E0023");
    assert!(explain.status.success());
    let explain_stdout = String::from_utf8_lossy(&explain.stdout);
    assert!(explain_stdout.contains("E0023 — Integer literal overflow"));
    assert!(explain_stdout.contains("9223372036854775807"));

    let list = Command::new(bin())
        .args(["errors", "list"])
        .output()
        .expect("run rz errors list");
    assert!(list.status.success());
    assert!(String::from_utf8_lossy(&list.stdout).contains("E0023"));
}
