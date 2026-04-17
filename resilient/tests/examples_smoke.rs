//! Smoke tests that invoke the compiled `resilient` binary against
//! files in `examples/`. After RES-003 (`println` builtin) `hello.rs`
//! runs end-to-end, so we now assert on actual stdout.

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
fn hello_rs_prints_greeting() {
    let (stdout, stderr, _code) = run_example("hello.rs");
    assert!(
        !stderr.contains("Parser error"),
        "unexpected parser error:\n{stderr}"
    );
    assert!(
        stdout.contains("Hello, Resilient world!"),
        "expected greeting in stdout, got:\nstdout={stdout}\nstderr={stderr}"
    );
}

#[test]
fn minimal_rs_calls_println_successfully() {
    // RES-003 criterion: minimal.rs must no longer fail on an undefined
    // `println`. It still fails later on `"msg" + int` (string + int
    // coercion is not in scope — that's a follow-up ticket), so we
    // assert on the first `println` call having worked.
    let (stdout, stderr, _code) = run_example("minimal.rs");
    assert!(
        !stderr.contains("Parser error"),
        "unexpected parser error:\n{stderr}"
    );
    assert!(
        !stderr.contains("Identifier not found: println"),
        "println is still undefined:\n{stderr}"
    );
    assert!(
        stdout.contains("Starting the program"),
        "expected first println output in stdout, got:\nstdout={stdout}\nstderr={stderr}"
    );
}
