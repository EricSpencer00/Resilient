//! RES-112: smoke test for the `--dump-tokens` driver flag.
//!
//! Spawns the `resilient` binary with `--dump-tokens
//! examples/hello.rs` and verifies the first three tokens are the
//! ones we expect for a vanilla `fn main() { ... }` opening. The
//! exact `Debug` form of each token comes from `Token`'s derive —
//! variant names here match the actual enum in `src/main.rs`, not
//! the short forms (`Fn` / `Ident` / `LParen`) the ticket sketch
//! used.

use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_resilient")
}

#[test]
fn dump_tokens_prints_first_three_tokens_of_hello() {
    let output = Command::new(bin())
        .arg("--dump-tokens")
        .arg("examples/hello.rs")
        .output()
        .expect("spawn resilient --dump-tokens");
    assert_eq!(
        output.status.code(),
        Some(0),
        "--dump-tokens must exit 0; stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let lines: Vec<&str> = stdout.lines().collect();
    assert!(
        lines.len() >= 3,
        "expected at least 3 token lines, got:\n{stdout}"
    );
    // First line — the `fn` keyword.
    assert!(
        lines[0].contains("Function") && lines[0].contains("\"fn\""),
        "line 1 should be `Function(\"fn\")`, got: {:?}",
        lines[0]
    );
    // Second line — the identifier `main`.
    assert!(
        lines[1].contains("Identifier") && lines[1].contains("main"),
        "line 2 should be `Identifier(\"main\")`, got: {:?}",
        lines[1]
    );
    // Third line — the opening paren.
    assert!(
        lines[2].contains("LeftParen"),
        "line 3 should be `LeftParen(\"(\")`, got: {:?}",
        lines[2]
    );
    // Final line — EOF sentinel.
    let last = lines.last().expect("at least one line");
    assert!(
        last.contains("Eof"),
        "last line should be `Eof`, got: {:?}",
        last
    );
}

#[test]
fn dump_tokens_rejects_mutually_exclusive_lsp() {
    let output = Command::new(bin())
        .arg("--dump-tokens")
        .arg("--lsp")
        .output()
        .expect("spawn resilient");
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("mutually exclusive"),
        "expected mutex diagnostic, got: {stderr}"
    );
}

#[test]
fn dump_tokens_without_path_errors_cleanly() {
    let output = Command::new(bin())
        .arg("--dump-tokens")
        .output()
        .expect("spawn resilient");
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("requires a path"),
        "expected missing-path diagnostic, got: {stderr}"
    );
}
