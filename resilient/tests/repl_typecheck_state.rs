//! RES-4445: typechecked REPL input must see the live session state.

use std::io::Write;
use std::process::{Command, Output, Stdio};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rz")
}

fn run_repl(input: &str) -> Output {
    let mut child = Command::new(bin())
        .arg("repl")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rz repl");

    child
        .stdin
        .take()
        .expect("repl stdin")
        .write_all(input.as_bytes())
        .expect("write REPL input");

    child.wait_with_output().expect("wait for rz repl")
}

#[test]
fn typecheck_preserves_let_bindings_across_inputs() {
    let output = run_repl("typecheck\nlet answer = 41;\nanswer + 1;\n");

    assert_eq!(
        output.status.code(),
        Some(0),
        "REPL should exit cleanly; stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stdout.contains("42"),
        "later input should evaluate the prior binding; stdout={stdout}"
    );
    assert!(
        !stderr.contains("Undefined variable answer"),
        "typecheck should retain prior bindings; stderr={stderr}"
    );
}

#[test]
fn typecheck_preserves_function_bindings_and_rejects_unknown_names() {
    let output = run_repl(
        "typecheck\nfn twice(int value) { return value * 2; }\ntwice(21);\nmissing_name;\n",
    );

    assert_eq!(
        output.status.code(),
        Some(0),
        "REPL should exit cleanly; stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stdout.contains("42"),
        "later input should call the prior function; stdout={stdout}"
    );
    assert!(
        stderr.contains("Undefined variable 'missing_name'"),
        "typecheck should still reject unknown names; stderr={stderr}"
    );
    assert!(
        !stdout.contains("Identifier not found: missing_name"),
        "unknown names must not execute after a typecheck failure; stdout={stdout}"
    );
}
