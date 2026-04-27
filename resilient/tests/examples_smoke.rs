//! Smoke tests that invoke the compiled `resilient` binary against
//! files in `examples/`. After RES-003 (`println` builtin) `hello.rs`
//! runs end-to-end, so we now assert on actual stdout.

use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rz")
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
    let (stdout, stderr, _code) = run_example("hello.rz");
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
    let (_s, _e, code) = run_example("hello.rz");
    assert_eq!(code, Some(0), "hello.rs should exit 0");
    let (_s, _e, code) = run_example("minimal.rz");
    assert_eq!(code, Some(0), "minimal.rs should exit 0");
}

#[test]
fn broken_example_exits_non_zero() {
    // sensor_example.rs has a parse error (parameterless fn w/o type).
    // Until someone fixes the example, running it must surface a
    // non-zero exit code so CI sees the failure.
    let (_s, _e, code) = run_example("sensor_example.rz");
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
        .arg("examples/cert_demo.rz")
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
fn bytecode_vm_runs_fn_call() {
    // RES-081: --vm runs a program that declares a fn and calls it.
    // Foundation only covers calls without branching — that's fine
    // for this smoke test; `sq(7) = 49` doesn't need control flow.
    use std::io::Write;
    let tmp = std::env::temp_dir().join(format!("res_081_smoke_{}.rs", std::process::id()));
    {
        let mut f = std::fs::File::create(&tmp).expect("create tmp");
        writeln!(f, "fn sq(int n) {{ return n * n; }}").unwrap();
        writeln!(f, "sq(7);").unwrap();
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
        "vm fn-call path must exit 0; stdout={stdout} stderr={stderr}"
    );
    assert!(
        stdout.contains("49"),
        "expected `49` in stdout (sq(7)); got:\n{stdout}"
    );
    let _ = std::fs::remove_file(&tmp);
}

#[test]
#[cfg(feature = "jit")]
fn bytecode_jit_runs_arithmetic_program() {
    // RES-096: end-to-end JIT path. Program is `return 7 + 14;`,
    // which the JIT lowers to native code and executes. Driver
    // prints the i64 result then exits 0.
    use std::io::Write;
    let tmp = std::env::temp_dir().join(format!("res_096_smoke_{}.rs", std::process::id()));
    {
        let mut f = std::fs::File::create(&tmp).expect("create tmp");
        writeln!(f, "return 7 + 14;").unwrap();
    }
    let output = Command::new(bin())
        .arg("--jit")
        .arg(&tmp)
        .output()
        .expect("spawn resilient --jit");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        output.status.code(),
        Some(0),
        "jit path must exit 0; stdout={stdout} stderr={stderr}"
    );
    assert!(
        stdout.contains("21"),
        "expected `21` in stdout (7 + 14 via JIT); got:\n{stdout}"
    );
    let _ = std::fs::remove_file(&tmp);
}

#[test]
#[cfg(feature = "jit")]
fn bytecode_jit_runs_division() {
    // RES-099: Phase C extends the JIT to Sub/Mul/Div/Mod.
    // `return 100 / 4;` exercises sdiv end-to-end (parse →
    // lower → cranelift → native code → driver prints).
    use std::io::Write;
    let tmp = std::env::temp_dir().join(format!("res_099_smoke_{}.rs", std::process::id()));
    {
        let mut f = std::fs::File::create(&tmp).expect("create tmp");
        writeln!(f, "return 100 / 4;").unwrap();
    }
    let output = Command::new(bin())
        .arg("--jit")
        .arg(&tmp)
        .output()
        .expect("spawn resilient --jit");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        output.status.code(),
        Some(0),
        "jit path must exit 0; stdout={stdout} stderr={stderr}"
    );
    assert!(
        stdout.contains("25"),
        "expected `25` in stdout (100 / 4 via JIT); got:\n{stdout}"
    );
    let _ = std::fs::remove_file(&tmp);
}

#[test]
#[cfg(feature = "jit")]
fn bytecode_jit_runs_comparison() {
    // RES-100: Phase D extends the JIT to icmp + bool literals.
    // `return 7 == 7;` exercises icmp + uextend → driver gets 1
    // back as i64. Joins the existing arith + division smokes in
    // covering the end-to-end JIT path for a third op family.
    use std::io::Write;
    let tmp = std::env::temp_dir().join(format!("res_100_smoke_{}.rs", std::process::id()));
    {
        let mut f = std::fs::File::create(&tmp).expect("create tmp");
        writeln!(f, "return 7 == 7;").unwrap();
    }
    let output = Command::new(bin())
        .arg("--jit")
        .arg(&tmp)
        .output()
        .expect("spawn resilient --jit");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        output.status.code(),
        Some(0),
        "jit path must exit 0; stdout={stdout} stderr={stderr}"
    );
    assert!(
        stdout.contains("1"),
        "expected `1` in stdout (7 == 7 via JIT); got:\n{stdout}"
    );
    let _ = std::fs::remove_file(&tmp);
}

#[test]
#[cfg(feature = "jit")]
fn bytecode_jit_runs_if_else() {
    // RES-102: Phase E adds if/else with cranelift brif. Program
    // takes the then-arm (3 < 7 is true) and exits via the
    // arm-local return. Exercises the full block dance:
    // brif → then_block → return_, plus the dead else_block →
    // return_ that the verifier still requires.
    use std::io::Write;
    let tmp = std::env::temp_dir().join(format!("res_102_smoke_{}.rs", std::process::id()));
    {
        let mut f = std::fs::File::create(&tmp).expect("create tmp");
        writeln!(f, "if (3 < 7) {{ return 42; }} else {{ return 0; }}").unwrap();
    }
    let output = Command::new(bin())
        .arg("--jit")
        .arg(&tmp)
        .output()
        .expect("spawn resilient --jit");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        output.status.code(),
        Some(0),
        "jit path must exit 0; stdout={stdout} stderr={stderr}"
    );
    assert!(
        stdout.contains("42"),
        "expected `42` in stdout (if/else then-arm via JIT); got:\n{stdout}"
    );
    let _ = std::fs::remove_file(&tmp);
}

#[test]
#[cfg(feature = "jit")]
fn bytecode_jit_runs_if_with_fallthrough() {
    // RES-103: Phase F lifts the both-arms-must-return rule by
    // adding a merge_block. `if (5 < 3) { return 7; } return 9;`
    // exercises the merge mechanic: condition false → bare-if
    // semantics via no-op else → fallthrough → trailing return.
    use std::io::Write;
    let tmp = std::env::temp_dir().join(format!("res_103_smoke_{}.rs", std::process::id()));
    {
        let mut f = std::fs::File::create(&tmp).expect("create tmp");
        writeln!(f, "if (5 < 3) {{ return 7; }} return 9;").unwrap();
    }
    let output = Command::new(bin())
        .arg("--jit")
        .arg(&tmp)
        .output()
        .expect("spawn resilient --jit");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        output.status.code(),
        Some(0),
        "jit path must exit 0; stdout={stdout} stderr={stderr}"
    );
    assert!(
        stdout.contains("9"),
        "expected `9` in stdout (fallthrough via JIT); got:\n{stdout}"
    );
    let _ = std::fs::remove_file(&tmp);
}

#[test]
#[cfg(feature = "jit")]
fn bytecode_jit_runs_let_bindings() {
    // RES-104: Phase G adds let bindings + identifier reads.
    // `let x = 100; let y = 4; return x / y;` exercises two
    // local variables flowing into a RES-099 sdiv. Driver gets
    // 25 back as i64.
    use std::io::Write;
    let tmp = std::env::temp_dir().join(format!("res_104_smoke_{}.rs", std::process::id()));
    {
        let mut f = std::fs::File::create(&tmp).expect("create tmp");
        writeln!(f, "let x = 100; let y = 4; return x / y;").unwrap();
    }
    let output = Command::new(bin())
        .arg("--jit")
        .arg(&tmp)
        .output()
        .expect("spawn resilient --jit");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        output.status.code(),
        Some(0),
        "jit path must exit 0; stdout={stdout} stderr={stderr}"
    );
    assert!(
        stdout.contains("25"),
        "expected `25` in stdout (let x/let y/return x/y via JIT); got:\n{stdout}"
    );
    let _ = std::fs::remove_file(&tmp);
}

#[test]
#[cfg(feature = "jit")]
fn bytecode_jit_runs_function_call() {
    // RES-105: Phase H adds user-defined function declarations
    // and direct calls. `fn double(int x) { return x + x; }
    // return double(21);` exercises the two-pass compilation:
    // declare double as a FuncId in Pass 1, compile its body in
    // Pass 2, lower the call site as `call(local_func_ref, &args)`.
    use std::io::Write;
    let tmp = std::env::temp_dir().join(format!("res_105_smoke_{}.rs", std::process::id()));
    {
        let mut f = std::fs::File::create(&tmp).expect("create tmp");
        writeln!(f, "fn double(int x) {{ return x + x; }} return double(21);").unwrap();
    }
    let output = Command::new(bin())
        .arg("--jit")
        .arg(&tmp)
        .output()
        .expect("spawn resilient --jit");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        output.status.code(),
        Some(0),
        "jit path must exit 0; stdout={stdout} stderr={stderr}"
    );
    assert!(
        stdout.contains("42"),
        "expected `42` in stdout (double(21) via JIT); got:\n{stdout}"
    );
    let _ = std::fs::remove_file(&tmp);
}

#[test]
fn vm_runtime_error_includes_source_filename() {
    // RES-095: the driver's --vm error path should prefix with
    // <file>:<line>: so editors auto-link the location, matching
    // the typechecker's RES-080 format.
    use std::io::Write;
    let tmp = std::env::temp_dir().join(format!("res_095_smoke_{}.rs", std::process::id()));
    {
        let mut f = std::fs::File::create(&tmp).expect("create tmp");
        // Line 1: fn opener; line 2: divide-by-zero; line 3: return; etc.
        writeln!(f, "fn boom(int n) {{").unwrap();
        writeln!(f, "    let r = 100 / n;").unwrap();
        writeln!(f, "    return r;").unwrap();
        writeln!(f, "}}").unwrap();
        writeln!(f, "boom(0);").unwrap();
    }
    let output = Command::new(bin())
        .arg("--vm")
        .arg(&tmp)
        .output()
        .expect("spawn resilient");
    let stderr = String::from_utf8_lossy(&output.stderr);
    let path_str = tmp.to_string_lossy();
    assert!(
        stderr.contains(path_str.as_ref()),
        "expected source path '{path_str}' in stderr; got:\n{stderr}"
    );
    assert!(
        stderr.contains(":2:"),
        "expected `:2:` line marker (the divide line) in stderr; got:\n{stderr}"
    );
    assert!(
        stderr.contains("divide by zero"),
        "expected divide-by-zero text; got:\n{stderr}"
    );
    assert_ne!(
        output.status.code(),
        Some(0),
        "VM runtime error must exit non-zero"
    );
    let _ = std::fs::remove_file(&tmp);
}

#[test]
fn bytecode_vm_runs_recursive_fib() {
    // RES-083: with control flow landed, fib becomes runnable under
    // --vm. This is the capstone smoke test that exercises Call +
    // ReturnFromCall + JumpIfFalse + back-patched forward/backward
    // offsets + comparison ops + recursion, all in one program.
    use std::io::Write;
    let tmp = std::env::temp_dir().join(format!("res_083_smoke_{}.rs", std::process::id()));
    {
        let mut f = std::fs::File::create(&tmp).expect("create tmp");
        writeln!(f, "fn fib(int n) {{").unwrap();
        writeln!(f, "    if n <= 1 {{ return n; }}").unwrap();
        writeln!(f, "    return fib(n - 1) + fib(n - 2);").unwrap();
        writeln!(f, "}}").unwrap();
        writeln!(f, "fib(10);").unwrap();
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
        "vm fib path must exit 0; stdout={stdout} stderr={stderr}"
    );
    assert!(
        stdout.contains("55"),
        "expected fib(10)=55 in stdout; got:\n{stdout}"
    );
    let _ = std::fs::remove_file(&tmp);
}

#[test]
fn bytecode_vm_rejects_unsupported_construct_cleanly() {
    // RES-076: anything outside the supported subset returns
    // `CompileError::Unsupported(...)` and the driver wraps it as
    // `VM compile error: ...` and exits non-zero. `for .. in` is
    // still out of scope after RES-083 — use it as the canary.
    use std::io::Write;
    let tmp = std::env::temp_dir().join(format!("res_076_unsupp_{}.rs", std::process::id()));
    {
        let mut f = std::fs::File::create(&tmp).expect("create tmp");
        writeln!(f, "for x in [1, 2, 3] {{ let y = x; }}").unwrap();
    }
    let output = Command::new(bin())
        .arg("--vm")
        .arg(&tmp)
        .output()
        .expect("spawn resilient");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_ne!(
        output.status.code(),
        Some(0),
        "unsupported VM input must fail"
    );
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
    assert_ne!(
        output.status.code(),
        Some(0),
        "type-check failure must exit non-zero"
    );
    let _ = std::fs::remove_file(&tmp);
}

#[test]
fn imports_demo_resolves_use_clause() {
    // RES-073: `use "helpers.rz";` in main.rz pulls in square() and
    // shout() so the program can call them as if they were declared
    // locally. Asserts both the imported function's stdout and the
    // imported helper's return value.
    let (stdout, stderr, code) = run_example("imports_demo/main.rz");
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
    let tmp = {
        use std::sync::atomic::{AtomicU64, Ordering};
        static CTR: AtomicU64 = AtomicU64::new(0);
        std::env::temp_dir().join(format!(
            "res_073_missing_use_{}_{}.rs",
            std::process::id(),
            CTR.fetch_add(1, Ordering::Relaxed),
        ))
    };
    {
        let mut f = std::fs::File::create(&tmp).expect("create tmp file");
        writeln!(
            f,
            "use \"definitely-not-here.rs\";\nfn main() {{}}\nmain();"
        )
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
#[cfg(feature = "jit")]
fn bytecode_jit_runs_while_loop() {
    // RES-107: Phase J adds reassignment + while loops. The
    // sum-loop from the ticket — `let i = 0; let sum = 0;
    // while (i < 5) { sum = sum + i; i = i + 1; } return sum;` —
    // must return 10 (0 + 1 + 2 + 3 + 4) through the JIT driver,
    // confirming the header/body/exit block dance compiles and
    // runs end-to-end.
    use std::io::Write;
    let tmp = std::env::temp_dir().join(format!("res_107_smoke_{}.rs", std::process::id()));
    {
        let mut f = std::fs::File::create(&tmp).expect("create tmp");
        writeln!(
            f,
            "let i = 0; let sum = 0; while (i < 5) {{ sum = sum + i; i = i + 1; }} return sum;"
        )
        .unwrap();
    }
    let output = Command::new(bin())
        .arg("--jit")
        .arg(&tmp)
        .output()
        .expect("spawn resilient --jit");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        output.status.code(),
        Some(0),
        "jit path must exit 0; stdout={stdout} stderr={stderr}"
    );
    assert!(
        stdout.contains("10"),
        "expected `10` from sum-loop through JIT; got:\n{stdout}"
    );
    let _ = std::fs::remove_file(&tmp);
}

/// RES-217: helper — write a temp program whose requires-clause is a
/// nonlinear integer predicate Z3 cannot resolve in 1ms. The contract
/// `a*a*a + b*b*b != 9*a*b*b + 3` (Mordell-curve-like form) forces the
/// NIA engine into a non-trivial search; with `--verifier-timeout-ms 1`
/// it reliably hits the timeout on every platform we run CI on.
#[cfg(feature = "z3")]
fn write_partial_proof_program(tag: &str) -> std::path::PathBuf {
    use std::io::Write;
    let tmp = std::env::temp_dir().join(format!("res_217_{}_{}.rs", tag, std::process::id()));
    let src = "\
fn check(int a, int b)
    requires a * a * a + b * b * b != 9 * a * b * b + 3
{
    return;
}

fn main() {
    check(1, 2);
}

main();
";
    let mut f = std::fs::File::create(&tmp).expect("create tmp");
    f.write_all(src.as_bytes()).unwrap();
    tmp
}

#[test]
#[cfg(feature = "z3")]
fn partial_proof_warning_fires_on_z3_timeout() {
    // RES-217: when Z3 returns Unknown (here forced by a small budget
    // on a nonlinear-int obligation), the typechecker must emit the
    // structured `warning[partial-proof]:` line pointing at the
    // specific assertion's source position. Compilation still
    // succeeds and the runtime check is retained.
    //
    // RES-399: bumped from 1ms → 100ms. 1ms is below Z3's internal
    // timeout-check granularity in some configurations (issue #268),
    // and the test occasionally hung the CI runner indefinitely. 100ms
    // is still well below the "Z3 actually solves Mordell" wall on
    // every platform we run, so the obligation reliably comes back
    // Unknown — but Z3 sees the deadline now.
    let tmp = write_partial_proof_program("timeout");
    let output = Command::new(bin())
        .arg("--typecheck")
        .arg("--verifier-timeout-ms")
        .arg("100")
        .arg(&tmp)
        .output()
        .expect("spawn resilient --typecheck");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("warning[partial-proof]:"),
        "expected partial-proof warning in stderr; got:\n{stderr}"
    );
    assert!(
        stderr.contains("Z3 returned Unknown for assertion at"),
        "expected canonical warning body; got:\n{stderr}"
    );
    assert!(
        stderr.contains("proof is incomplete"),
        "expected warning suffix; got:\n{stderr}"
    );
    // File path and line:col should be present (not `<unknown>`).
    let tmp_str = tmp.to_string_lossy();
    assert!(
        stderr.contains(tmp_str.as_ref()),
        "expected source path in warning; got:\n{stderr}"
    );
    let _ = std::fs::remove_file(&tmp);
}

#[test]
#[cfg(feature = "z3")]
fn no_warn_unverified_suppresses_partial_proof_warning() {
    // RES-217: `--no-warn-unverified` silences the structured
    // warning even when Z3 times out. The per-fn `hint:` line
    // (pre-RES-217 diagnostic) is still emitted — the two are
    // intentionally independent signals.
    //
    // RES-399: bumped from 1ms → 100ms (issue #268). This test's
    // sibling above passed at 1ms but THIS one hung indefinitely on
    // both macOS local dev and Ubuntu CI — the `--no-warn-unverified`
    // path apparently took a different code branch that didn't honor
    // the microsecond-scale deadline. 100ms is still small enough that
    // the Mordell obligation comes back Unknown, but Z3's solver loop
    // sees the deadline reliably.
    let tmp = write_partial_proof_program("suppressed");
    let output = Command::new(bin())
        .arg("--typecheck")
        .arg("--verifier-timeout-ms")
        .arg("100")
        .arg("--no-warn-unverified")
        .arg(&tmp)
        .output()
        .expect("spawn resilient --typecheck --no-warn-unverified");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("warning[partial-proof]:"),
        "partial-proof warning should be suppressed; got:\n{stderr}"
    );
    let _ = std::fs::remove_file(&tmp);
}

#[test]
fn minimal_rs_runs_end_to_end() {
    // After RES-003 (println) and RES-008 (string+primitive coercion)
    // minimal.rs runs to completion.
    let (stdout, stderr, _code) = run_example("minimal.rz");
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

#[test]
#[cfg(all(feature = "ffi", target_os = "linux"))]
fn ffi_libm_example_calls_sqrt() {
    // Smoke-test the ffi_libm.rs example end-to-end on Linux where
    // libm.so.6 is always available. Checks sqrt(16.0)=4 and sqrt(2.0)
    // round-trips through the C trampoline. Gated on feature=ffi and
    // target_os=linux; skipped silently on other platforms.
    let (stdout, stderr, code) = run_example("ffi_libm.rz");
    assert_eq!(
        code,
        Some(0),
        "ffi_libm.rs must exit 0; stdout={stdout} stderr={stderr}"
    );
    assert!(
        stdout.lines().any(|l| l.trim() == "4"),
        "expected a line `4` from sqrt(16.0) in stdout; got:\nstdout={stdout}\nstderr={stderr}"
    );
    assert!(
        stdout.contains("1.4142135623730951"),
        "expected sqrt(2.0) in stdout; got:\nstdout={stdout}\nstderr={stderr}"
    );
}

#[test]
#[cfg(feature = "z3")]
fn actor_commute_example_proves_handlers_commute() {
    // RES-386: the Counter actor's `increment` and `decrement`
    // handlers both rewrite `self.state` as `state + k`; Z3 proves
    // (state + 1) - 1 == (state - 1) + 1 for all integers, so the
    // verifier must print a `commute` diagnostic and the driver must
    // exit 0. This exercises the entire commutativity pipeline
    // end-to-end — parser → driver → verifier_z3::check_actor_commutativity.
    let (stdout, stderr, code) = run_example("actor_commute.rz");
    assert_eq!(
        code,
        Some(0),
        "actor_commute.rz must exit 0; stdout={stdout} stderr={stderr}"
    );
    assert!(
        stdout.contains("verifier: actor Counter: handlers `increment` and `decrement` commute"),
        "expected commutativity diagnostic in stdout; got:\n{stdout}"
    );
    assert!(
        !stdout.contains("do not commute"),
        "commutative pair should not produce a divergence diagnostic; got:\n{stdout}"
    );
}

#[test]
#[cfg(feature = "z3")]
fn actor_noncommute_example_reports_counterexample() {
    // RES-386: the Accumulator actor's `inc` (state + 1) and `double`
    // (state * 2) handlers do not commute — starting from state = 0,
    // inc-then-double yields 2 while double-then-inc yields 1. The
    // verifier prints the counterexample line with both final states,
    // and the driver still exits 0 (verifier diagnostics are advisory,
    // not fatal, per the minimum-slice contract).
    let (stdout, stderr, code) = run_example("actor_noncommute.rz");
    assert_eq!(
        code,
        Some(0),
        "actor_noncommute.rz must exit 0; stdout={stdout} stderr={stderr}"
    );
    assert!(
        stdout.contains("verifier: actor Accumulator: handlers `inc` and `double` do not commute"),
        "expected divergence diagnostic in stdout; got:\n{stdout}"
    );
    assert!(
        stdout.contains("counterexample:"),
        "expected counterexample marker in stdout; got:\n{stdout}"
    );
    // The verifier formats each final state as `a_then_b=<int>` and
    // `b_then_a=<int>`. We check for both substrings rather than
    // literal integers so the assertion tolerates any Z3 model choice
    // (the solver is free to pick any state_0 that falsifies the
    // commutativity query).
    assert!(
        stdout.contains("inc_then_double=") && stdout.contains("double_then_inc="),
        "expected both order-specific final states in stdout; got:\n{stdout}"
    );
}

#[test]
#[cfg(feature = "z3")]
fn bitmask_extract_example_runs_and_produces_nibble() {
    // RES-354: bitmask_extract.rz exercises the bitwise-AND operator.
    // The function `extract_nibble(255)` should return 15 (0xFF & 0xF)
    // and `extract_nibble(0)` should return 0. The binary must exit 0
    // and the BV32 path in the verifier must not crash.
    let (stdout, stderr, code) = run_example("bitmask_extract.rz");
    assert_eq!(
        code,
        Some(0),
        "bitmask_extract.rz must exit 0; stdout={stdout} stderr={stderr}"
    );
    assert!(
        !stderr.contains("Parser error"),
        "unexpected parser error:\n{stderr}"
    );
    // `extract_nibble(255)` returns 255 & 15 = 15.
    assert!(
        stdout.contains("15"),
        "expected nibble 15 in stdout; got:\n{stdout}"
    );
    // `extract_nibble(0)` returns 0 & 15 = 0.
    assert!(
        stdout.contains("0"),
        "expected nibble 0 in stdout; got:\n{stdout}"
    );
}

#[test]
#[cfg(feature = "z3")]
fn shift_bounds_example_runs_and_shifts_correctly() {
    // RES-354: shift_bounds.rz exercises the shift-right (`>>`) and
    // XOR (`^`) operators. The function `shift_and_xor(256)` returns
    // 256 >> 4 ^ 0 = 16, and `shift_and_xor(16)` returns 1.
    // The binary must exit 0 and the BV32 path must not crash.
    let (stdout, stderr, code) = run_example("shift_bounds.rz");
    assert_eq!(
        code,
        Some(0),
        "shift_bounds.rz must exit 0; stdout={stdout} stderr={stderr}"
    );
    assert!(
        !stderr.contains("Parser error"),
        "unexpected parser error:\n{stderr}"
    );
    // `shift_and_xor(256)` = (256 >> 4) ^ 0 = 16 ^ 0 = 16.
    assert!(
        stdout.contains("16"),
        "expected 16 in stdout; got:\n{stdout}"
    );
    // `shift_and_xor(16)` = (16 >> 4) ^ 0 = 1 ^ 0 = 1.
    assert!(stdout.contains("1"), "expected 1 in stdout; got:\n{stdout}");
}

#[test]
fn bytecode_vm_runs_println_builtin() {
    // RES-VM (issue #266): the bytecode VM previously rejected any
    // call to a builtin (`println`, `len`, `to_upper`, ...) with
    // `bytecode compile: unknown function: println`. Wire-up: the
    // compiler emits `Op::CallBuiltin { name_const, arity }` for any
    // call site whose callee isn't a user-defined fn or a foreign
    // symbol, and the VM dispatches it through the same `BUILTINS`
    // table the tree walker uses.
    use std::io::Write;
    let tmp = std::env::temp_dir().join(format!("res_vm_println_{}.rs", std::process::id()));
    {
        let mut f = std::fs::File::create(&tmp).expect("create tmp");
        writeln!(f, "fn main() {{ println(\"hello-from-vm\"); return 0; }}").unwrap();
        writeln!(f, "main();").unwrap();
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
        "vm println path must exit 0; stdout={stdout} stderr={stderr}"
    );
    assert!(
        !stderr.contains("unknown function"),
        "regression: builtin lookup failed under --vm; stderr=\n{stderr}"
    );
    assert!(
        stdout.contains("hello-from-vm"),
        "expected `hello-from-vm` in stdout; got:\n{stdout}"
    );
    let _ = std::fs::remove_file(&tmp);
}

#[test]
fn bytecode_vm_runs_multiple_builtins() {
    // RES-VM (issue #266): exercise three commonly-used builtins
    // through the VM in a single program. `len` returns an i64 that
    // round-trips back to the println dispatch; `to_upper` returns a
    // String the VM must keep on the operand stack until the next
    // `println` consumes it. Catches regressions where the dispatch
    // arm forgets to push the result, or where the constant-pool
    // name interning collides across distinct call sites.
    use std::io::Write;
    let tmp = std::env::temp_dir().join(format!("res_vm_builtins_{}.rs", std::process::id()));
    {
        let mut f = std::fs::File::create(&tmp).expect("create tmp");
        writeln!(f, "println(\"hi\");").unwrap();
        writeln!(f, "println(to_upper(\"resilient\"));").unwrap();
        writeln!(f, "let s = \"hello\";").unwrap();
        writeln!(f, "println(len(s));").unwrap();
        writeln!(f, "return 0;").unwrap();
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
        "vm multi-builtin path must exit 0; stdout={stdout} stderr={stderr}"
    );
    assert!(
        stdout.contains("hi") && stdout.contains("RESILIENT") && stdout.contains("5"),
        "expected hi / RESILIENT / 5 from println+to_upper+len; got:\n{stdout}"
    );
    let _ = std::fs::remove_file(&tmp);
}

#[test]
fn bytecode_vm_runs_clamp_and_atan2() {
    // RES-295: the new math builtins (clamp, atan2) must dispatch
    // through `Op::CallBuiltin` exactly like every other entry in
    // `BUILTINS`. Acceptance criterion: "they also work under --vm
    // (the new CallBuiltin path makes this automatic IF you register
    // them in the canonical BUILTINS table)" — this test pins down
    // that registration so a future revert of the registration line
    // produces a red CI signal, not a silent VM-only regression.
    use std::io::Write;
    let tmp = std::env::temp_dir().join(format!("res_vm_res295_{}.rs", std::process::id()));
    {
        let mut f = std::fs::File::create(&tmp).expect("create tmp");
        writeln!(f, "println(clamp(15, 0, 10));").unwrap();
        writeln!(f, "println(clamp(-3, 0, 10));").unwrap();
        writeln!(f, "println(atan2(0.0, 1.0));").unwrap();
        writeln!(f, "return 0;").unwrap();
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
        "vm clamp/atan2 path must exit 0; stdout={stdout} stderr={stderr}"
    );
    assert!(
        !stderr.contains("unknown function"),
        "regression: clamp/atan2 lookup failed under --vm; stderr=\n{stderr}"
    );
    // clamp(15, 0, 10) = 10; clamp(-3, 0, 10) = 0; atan2(0, 1) = 0.
    assert!(
        stdout.contains("10") && stdout.contains("0"),
        "expected 10 and 0 in stdout; got:\n{stdout}"
    );
    let _ = std::fs::remove_file(&tmp);
}
