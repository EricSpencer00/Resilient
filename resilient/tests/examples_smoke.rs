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
fn hello_exits_zero_minimal_exits_zero() {
    // RES-027: successful runs produce exit code 0.
    let (_s, _e, code) = run_example("hello.rs");
    assert_eq!(code, Some(0), "hello.rs should exit 0");
    let (_s, _e, code) = run_example("minimal.rs");
    assert_eq!(code, Some(0), "minimal.rs should exit 0");
}

#[test]
fn broken_example_exits_non_zero() {
    // sensor_example.rs has a parse error (parameterless fn w/o type).
    // Until someone fixes the example, running it must surface a
    // non-zero exit code so CI sees the failure.
    let (_s, _e, code) = run_example("sensor_example.rs");
    assert_ne!(code, Some(0), "broken example should NOT exit 0");
}

#[test]
fn imports_demo_resolves_use_clause() {
    // RES-073: `use "helpers.rs";` in main.rs pulls in square() and
    // shout() so the program can call them as if they were declared
    // locally. Asserts both the imported function's stdout and the
    // imported helper's return value.
    let (stdout, stderr, code) = run_example("imports_demo/main.rs");
    assert!(
        !stderr.contains("Parser error") && !stderr.contains("Import error"),
        "unexpected error:\nstderr={stderr}"
    );
    assert!(
        stdout.contains("imports work"),
        "expected shout() output, got:\n{stdout}"
    );
    assert!(
        stdout.contains("49"),
        "expected square(7) = 49 in output, got:\n{stdout}"
    );
    assert_eq!(code, Some(0), "imports demo must exit 0");
}

#[test]
fn imports_missing_file_errors_cleanly() {
    // RES-073: a `use "missing.rs";` against a non-existent path must
    // produce a clean diagnostic and a non-zero exit, not a panic.
    use std::io::Write;
    let tmp = std::env::temp_dir().join("res_073_missing_use.rs");
    {
        let mut f = std::fs::File::create(&tmp).expect("create tmp file");
        writeln!(f, "use \"definitely-not-here.rs\";\nfn main() {{}}\nmain();")
            .expect("write tmp");
    }
    let output = Command::new(bin())
        .arg(&tmp)
        .output()
        .expect("spawn resilient");
    let stderr = String::from_utf8_lossy(&output.stderr);
    let code = output.status.code();
    assert_ne!(code, Some(0), "missing import must fail; stderr={stderr}");
    assert!(
        stderr.contains("Import error") || stderr.contains("could not be resolved"),
        "expected import-error diagnostic, got:\n{stderr}"
    );
    let _ = std::fs::remove_file(&tmp);
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
