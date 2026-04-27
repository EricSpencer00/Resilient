//! RES-309: differential testing — interpreter vs VM (vs JIT).
//!
//! The interpreter, bytecode VM, and JIT are three independent execution
//! engines. A divergence — a program that prints `42` on one and `0` on
//! another — can hide silently for months without a test that runs the
//! same source through more than one backend. This file is that test.
//!
//! ## What's covered today
//!
//! - **Tree walker** (default driver) vs **bytecode VM** (`--vm`) for a
//!   curated list of `examples/*.rz` programs.
//! - The list is explicit and additive — see [`SHARED_EXAMPLES`] below.
//!   We do NOT iterate every example because some intentionally exercise
//!   tree-walker-only language features (string + int coercion, indirect
//!   calls, closures, list comprehensions, nested mutation). Those would
//!   produce a noisy red signal that obscures the real divergences this
//!   harness is here to catch.
//!
//! ## What's not covered yet (deliberate)
//!
//! - The JIT backend (`--features jit`) is **out of scope for this PR**.
//!   The JIT lowering only supports a small i64-only subset (no
//!   strings, no structs, no actors, no Z3-discharged contracts) so
//!   running it against the same example list would force every program
//!   through the "skip — unimplemented JIT node" path. See follow-up
//!   ticket: a JIT-only differential pass once the lowering covers
//!   enough surface to be meaningful.
//!
//! ## Why we strip stderr
//!
//! Both backends print a `seed=<u64>` line to stderr from the runtime
//! fault-injection harness. The seed is non-deterministic across runs
//! AND across backends (each path samples the RNG independently). So
//! the differential check compares **stdout exactly** and **exit code
//! exactly** — stderr is informational only.
//!
//! ## Catching divergence in the framework itself
//!
//! [`compare_outputs`] is the comparison primitive. We unit-test it
//! directly with synthesised divergent transcripts so a regression in
//! the *checker itself* (e.g. accidentally normalising whitespace,
//! ignoring exit codes) is caught even if no example happens to
//! diverge between backends.

use std::path::Path;
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rz")
}

/// One run of the driver: stdout, exit code. Stderr is intentionally
/// dropped — it carries the non-deterministic `seed=...` line and any
/// runtime diagnostics that aren't part of the "program semantics"
/// contract this differential check is asserting on.
struct Run {
    stdout: String,
    code: Option<i32>,
}

fn run_interpreter(example: &str) -> Run {
    let path = format!("examples/{example}");
    let output = Command::new(bin())
        .arg(&path)
        .output()
        .expect("failed to spawn rz (interpreter)");
    Run {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        code: output.status.code(),
    }
}

fn run_vm(example: &str) -> Run {
    let path = format!("examples/{example}");
    let output = Command::new(bin())
        .arg("--vm")
        .arg(&path)
        .output()
        .expect("failed to spawn rz --vm");
    Run {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        code: output.status.code(),
    }
}

/// Result of comparing two backend runs. `Ok(())` means the two agree;
/// `Err(diff)` carries a multi-line, copy-pasteable diff suitable for
/// `assert!(.., "{diff}")`.
fn compare_outputs(label_a: &str, a: &Run, label_b: &str, b: &Run) -> Result<(), String> {
    if a.stdout == b.stdout && a.code == b.code {
        return Ok(());
    }
    let mut msg = String::new();
    msg.push_str("backend disagreement:\n");
    if a.code != b.code {
        msg.push_str(&format!(
            "  exit-code: {label_a}={:?}  vs  {label_b}={:?}\n",
            a.code, b.code
        ));
    }
    if a.stdout != b.stdout {
        msg.push_str("  stdout disagreement:\n");
        msg.push_str(&format!("    --- {label_a} stdout ---\n"));
        for line in a.stdout.lines() {
            msg.push_str("    ");
            msg.push_str(line);
            msg.push('\n');
        }
        msg.push_str(&format!("    --- {label_b} stdout ---\n"));
        for line in b.stdout.lines() {
            msg.push_str("    ");
            msg.push_str(line);
            msg.push('\n');
        }
    }
    Err(msg)
}

/// Curated list of examples that the bytecode VM has been verified to
/// support (as of RES-309). Each entry must produce identical stdout
/// and the same exit code on the tree walker and the VM.
///
/// **Adding to this list** — pick any `examples/*.rz` whose feature
/// set is supported by the VM bytecode compiler. The VM rejects
/// `Const` declarations, `string + int` coercion, indirect calls,
/// nested-index assignment, and a handful of other tree-walker-only
/// constructs with `bytecode compile: unsupported construct: ...`.
/// Programs that hit those will fail this differential check; either
/// they belong here or they don't.
const SHARED_EXAMPLES: &[&str] = &[
    "hello.rz",
    "int_math.rz",
    "pinned_int_types.rz",
    "array_bounds_proven.rz",
    "array_bounds_runtime.rz",
    "fault_model_demo.rz",
    "clock_builtin.rz",
    "cert_demo.rz",
    "recovers_to_ok.rz",
    "region_distinct_ok.rz",
    "shebang_demo.rz",
    "actor_eventually_drain.rz",
];

#[test]
fn shared_examples_exist() {
    // Cheap pre-flight: if someone deletes an example from disk the
    // outer differential test will fail with a less obvious "spawn
    // succeeded but output is empty" error. Surface the cause early.
    for example in SHARED_EXAMPLES {
        let path = format!("examples/{example}");
        assert!(
            Path::new(&path).exists(),
            "SHARED_EXAMPLES references missing file: {path}"
        );
    }
}

#[test]
fn at_least_ten_shared_examples_are_pinned() {
    // RES-309 acceptance criterion: "at least ten existing examples
    // are covered by the differential run." Pin it as a test so a
    // future code-cleanup can't silently shrink the matrix.
    assert!(
        SHARED_EXAMPLES.len() >= 10,
        "differential matrix has only {} entries — RES-309 requires \u{2265} 10",
        SHARED_EXAMPLES.len()
    );
}

#[test]
fn interpreter_and_vm_agree_on_shared_examples() {
    let mut failures: Vec<String> = Vec::new();
    for example in SHARED_EXAMPLES {
        let interp = run_interpreter(example);
        let vm = run_vm(example);
        if let Err(diff) = compare_outputs("interpreter", &interp, "vm", &vm) {
            failures.push(format!("\n--- {example} ---\n{diff}"));
        }
    }
    assert!(
        failures.is_empty(),
        "{} backend(s) diverged:{}",
        failures.len(),
        failures.join("")
    );
}

#[test]
fn compare_outputs_detects_stdout_divergence() {
    // Sanity-check the comparison primitive. If a future refactor
    // accidentally makes `compare_outputs` lenient (say, trims
    // whitespace or ignores a trailing newline) the divergence-
    // detection part of this harness silently stops working — even
    // though the example matrix is green. This unit test pins the
    // detection itself.
    let a = Run {
        stdout: "42\n".to_string(),
        code: Some(0),
    };
    let b = Run {
        stdout: "0\n".to_string(),
        code: Some(0),
    };
    let result = compare_outputs("a", &a, "b", &b);
    let err = result.expect_err("must flag stdout disagreement");
    assert!(err.contains("stdout disagreement"));
    assert!(err.contains("42"));
    assert!(err.contains("0"));
}

#[test]
fn compare_outputs_detects_exit_code_divergence() {
    let a = Run {
        stdout: "same\n".to_string(),
        code: Some(0),
    };
    let b = Run {
        stdout: "same\n".to_string(),
        code: Some(1),
    };
    let result = compare_outputs("interp", &a, "vm", &b);
    let err = result.expect_err("must flag exit-code disagreement");
    assert!(err.contains("exit-code"));
    assert!(err.contains("Some(0)"));
    assert!(err.contains("Some(1)"));
}

#[test]
fn compare_outputs_accepts_identical_runs() {
    let a = Run {
        stdout: "identical\n".to_string(),
        code: Some(0),
    };
    let b = Run {
        stdout: "identical\n".to_string(),
        code: Some(0),
    };
    assert!(compare_outputs("a", &a, "b", &b).is_ok());
}
