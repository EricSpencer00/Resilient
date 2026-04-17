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
#[cfg(feature = "z3")]
fn emit_certificate_writes_reverifiable_smt2() {
    // RES-071: --emit-certificate <DIR> dumps an SMT-LIB2 file per
    // Z3-discharged contract obligation. Each file, fed back to stock
    // Z3, must report `unsat` (which is the proof). The test skips
    // cleanly if `z3` is not on PATH — no assumption about the CI
    // environment.
    let tmp = std::env::temp_dir().join(format!("res_071_smoke_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);

    let output = Command::new(bin())
        .arg("--emit-certificate")
        .arg(&tmp)
        .arg("examples/cert_demo.rs")
        .output()
        .expect("spawn resilient");
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        output.status.code(),
        Some(0),
        "driver should exit 0; stdout={stdout} stderr={stderr}"
    );
    assert!(
        stdout.contains("Wrote 1 verification certificate"),
        "expected cert-emission line; got:\n{stdout}"
    );

    // At least one .smt2 file landed.
    let entries: Vec<_> = std::fs::read_dir(&tmp)
        .expect("certificate dir was not created")
        .flatten()
        .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("smt2"))
        .collect();
    assert!(!entries.is_empty(), "no .smt2 cert in {}", tmp.display());

    // Re-verify with stock Z3 if available; if not, skip cleanly.
    let z3_present = Command::new("z3").arg("-version").output().is_ok();
    if z3_present {
        for entry in &entries {
            let out = Command::new("z3")
                .arg("-smt2")
                .arg(entry.path())
                .output()
                .expect("spawn stock z3");
            let stdout = String::from_utf8_lossy(&out.stdout);
            assert!(
                stdout.contains("unsat"),
                "stock z3 did not return unsat for {}; got: {stdout}",
                entry.path().display()
            );
        }
    } else {
        eprintln!("(z3 binary not on PATH — skipping re-verification step)");
    }

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn bytecode_vm_runs_arithmetic_and_let() {
    // RES-076: --vm routes the program through the bytecode VM
    // instead of the tree-walking interpreter. The same result is
    // printed; this proves the foundation pipeline (compile + run)
    // works end-to-end for the subset the FOUNDATION ticket covers.
    use std::io::Write;
    let tmp = std::env::temp_dir().join(format!("res_076_smoke_{}.rs", std::process::id()));
    {
        let mut f = std::fs::File::create(&tmp).expect("create tmp");
        writeln!(f, "let x = 2 + 3 * 4;").unwrap();
        writeln!(f, "return x;").unwrap();
    }
    let output = Command::new(bin())
        .arg("--vm")
        .arg(&tmp)
        .output()
        .expect("spawn resilient");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        output.status.code(),
        Some(0),
        "vm path must exit 0; stdout={stdout} stderr={stderr}"
    );
    assert!(
        stdout.contains("14"),
        "expected `14` in stdout (2 + 3 * 4); got:\n{stdout}"
    );
    let _ = std::fs::remove_file(&tmp);
}

#[test]
fn bytecode_vm_rejects_unsupported_construct_cleanly() {
    // RES-076: anything outside the FOUNDATION subset (e.g. `if`)
    // returns `CompileError::Unsupported(...)` and the driver
    // wraps it as `VM compile error: ...` and exits non-zero.
    use std::io::Write;
    let tmp = std::env::temp_dir().join(format!("res_076_unsupp_{}.rs", std::process::id()));
    {
        let mut f = std::fs::File::create(&tmp).expect("create tmp");
        writeln!(f, "if true {{ let x = 1; }}").unwrap();
    }
    let output = Command::new(bin())
        .arg("--vm")
        .arg(&tmp)
        .output()
        .expect("spawn resilient");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_ne!(output.status.code(), Some(0), "unsupported VM input must fail");
    assert!(
        stderr.contains("VM compile error") || stderr.contains("unsupported"),
        "expected VM compile-error diagnostic; got:\n{stderr}"
    );
    let _ = std::fs::remove_file(&tmp);
}

#[test]
fn typecheck_error_prefixes_path_and_line() {
    // RES-080: --typecheck on a file with a type error on line 3
    // must produce stderr containing `<tempfile>:3:` prefix so users
    // can navigate straight to the offending statement.
    use std::io::Write;
    let tmp = std::env::temp_dir().join(format!("res_080_smoke_{}.rs", std::process::id()));
    {
        let mut f = std::fs::File::create(&tmp).expect("create tmp");
        writeln!(f, "let a = 1;").unwrap();
        writeln!(f, "let b = 2;").unwrap();
        writeln!(f, "let bad: int = \"not an int\";").unwrap();
    }
    let output = Command::new(bin())
        .arg("--typecheck")
        .arg(&tmp)
        .output()
        .expect("spawn resilient");
    let stderr = String::from_utf8_lossy(&output.stderr);
    let path_str = tmp.to_string_lossy();
    // The :3: line marker must appear AND the file name must be present.
    assert!(
        stderr.contains(":3:"),
        "expected `:3:` line marker in stderr; got:\n{stderr}"
    );
    assert!(
        stderr.contains(path_str.as_ref()),
        "expected source path '{path_str}' in stderr; got:\n{stderr}"
    );
    assert_ne!(output.status.code(), Some(0), "type-check failure must exit non-zero");
    let _ = std::fs::remove_file(&tmp);
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
