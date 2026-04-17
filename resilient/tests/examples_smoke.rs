//! Smoke tests that invoke the compiled `resilient` binary against
//! files in `examples/`. For now the runtime is not complete enough
//! to actually execute `hello.rs` end-to-end (no `println` builtin —
//! see RES-003), so we assert on observable parse behavior:
//!
//! - The binary reaches the interpretation stage (not a parse error)
//! - It surfaces the expected runtime error about `println`
//!
//! Once RES-003 lands, tests here graduate to output assertions.

use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_resilient")
}

fn run_example(name: &str) -> (String, String, Option<i32>) {
    let path = format!("examples/{name}");
    let output = Command::new(bin())
        .arg(&path)
        .output()
        .expect("failed to spawn resilient binary");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn hello_rs_parses_to_a_program() {
    // hello.rs is the smallest example. With RES-001 (lexer fix) the
    // parser must get past `fn main(int dummy) {`. Without RES-003
    // (println builtin) the interpreter fails on the call. This test
    // flips the signal: parse failure shows as "Parser error:" on
    // stderr; a runtime error about `println` is the *success* signal.
    let (_stdout, stderr, _code) = run_example("hello.rs");
    assert!(
        !stderr.contains("Parser error"),
        "unexpected parser error:\n{stderr}"
    );
    assert!(
        stderr.contains("println") || stderr.contains("Identifier not found"),
        "expected runtime error about undefined println, got:\n{stderr}"
    );
}

#[test]
fn minimal_rs_parses_to_a_program() {
    // Same idea as hello_rs. minimal.rs defines and calls add_one.
    let (_stdout, stderr, _code) = run_example("minimal.rs");
    assert!(
        !stderr.contains("Parser error"),
        "unexpected parser error:\n{stderr}"
    );
}
