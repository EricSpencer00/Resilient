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
fn minimal_rs_runs_end_to_end() {
    // After RES-003 (println) and RES-008 (string+primitive coercion)
    // minimal.rs runs to completion.
    let (stdout, stderr, _code) = run_example("minimal.rs");
    assert!(
        !stderr.contains("Parser error"),
        "unexpected parser error:\n{stderr}"
    );
    assert!(
        stdout.contains("Starting the program"),
        "missing starting println:\n{stdout}"
    );
    assert!(
        stdout.contains("The answer is: 42"),
        "expected coerced concatenation result, got:\nstdout={stdout}\nstderr={stderr}"
    );
    assert!(
        stdout.contains("Program completed"),
        "missing completion println:\n{stdout}"
    );
}
