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
    "vm_stdlib_math.rz",
    // RES-3889: string subscript `s[i]` must yield a `Char` (not a
    // single-char string) on both backends so `s[i] == 'c'` and
    // `"x" + s[i]` agree.
    "string_subscript_char.rz",
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

/// Run inline `src` on one backend by staging it in a unique temp file.
/// Mirrors [`run_interpreter`] / [`run_vm`] but lets a test pin a tiny
/// program without adding an `examples/*.rz` sidecar.
fn run_src(src: &str, tag: &str, vm: bool) -> Run {
    use std::sync::atomic::{AtomicU32, Ordering};
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "rz_differential_{tag}_{}_{n}.rz",
        std::process::id()
    ));
    std::fs::write(&path, src).expect("write temp source");
    let mut cmd = Command::new(bin());
    if vm {
        cmd.arg("--vm");
    }
    let output = cmd.arg(&path).output().expect("failed to spawn rz");
    let _ = std::fs::remove_file(&path);
    Run {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        code: output.status.code(),
    }
}

/// RES-3891: cross-kind `==` / `!=` must behave identically on both
/// backends. The tree walker raises a runtime type mismatch; before the
/// fix the VM silently reported the operands unequal (`false`) and kept
/// running, so identical source produced different stdout *and* a
/// different exit code. Lock the class here so it can't silently return.
#[test]
fn interpreter_and_vm_agree_on_cross_type_equality() {
    // Each program compares operands of different kinds. Both backends
    // must agree on stdout and exit code (a type-mismatch error → the
    // `if`/`else` branch never prints and the process exits non-zero).
    let programs = [
        (
            "char_eq_int",
            "fn main() { let s = \"ab\"; let c = s[0]; if c == 5 { println(\"eq\"); } else { println(\"ne\"); } } main();",
        ),
        (
            "char_neq_int",
            "fn main() { let s = \"ab\"; let c = s[0]; if c != 5 { println(\"ne\"); } else { println(\"eq\"); } } main();",
        ),
        (
            "int_eq_string",
            "fn main() { let x = 1; let y = \"1\"; if x == y { println(\"eq\"); } else { println(\"ne\"); } } main();",
        ),
        (
            "int_eq_bool",
            "fn main() { let x = 1; let y = true; if x == y { println(\"eq\"); } else { println(\"ne\"); } } main();",
        ),
        (
            "int_eq_float",
            "fn main() { let x = 1; let y = 1.0; if x == y { println(\"eq\"); } else { println(\"ne\"); } } main();",
        ),
    ];
    let mut failures: Vec<String> = Vec::new();
    for (tag, src) in programs {
        let interp = run_src(src, tag, false);
        let vm = run_src(src, tag, true);
        if let Err(diff) = compare_outputs("interpreter", &interp, "vm", &vm) {
            failures.push(format!("\n--- {tag} ---\n{diff}"));
        }
    }
    assert!(
        failures.is_empty(),
        "{} cross-type comparison(s) diverged between backends:{}",
        failures.len(),
        failures.join("")
    );
}

/// RES-3894: `&&` / `||` with a non-bool operand must behave identically on
/// both backends. The tree walker raises a runtime type mismatch; before the
/// fix the VM coerced the operand via the truthiness rule and kept running,
/// so identical source produced different stdout *and* exit code. Lock the
/// class here so it can't silently return.
#[test]
fn interpreter_and_vm_agree_on_non_bool_logical_operands() {
    let programs = [
        (
            "and_int_left",
            "fn main() { if 5 && true { println(\"y\"); } else { println(\"n\"); } } main();",
        ),
        (
            "and_int_right",
            "fn main() { if true && 5 { println(\"y\"); } else { println(\"n\"); } } main();",
        ),
        (
            "and_zero_left",
            "fn main() { if 0 && true { println(\"y\"); } else { println(\"n\"); } } main();",
        ),
        (
            "and_string_left",
            "fn main() { if \"a\" && true { println(\"y\"); } else { println(\"n\"); } } main();",
        ),
        (
            "or_int_left",
            "fn main() { if 5 || false { println(\"y\"); } else { println(\"n\"); } } main();",
        ),
        (
            "or_int_right",
            "fn main() { if false || 5 { println(\"y\"); } else { println(\"n\"); } } main();",
        ),
        (
            "or_zero_left",
            "fn main() { if 0 || false { println(\"y\"); } else { println(\"n\"); } } main();",
        ),
    ];
    let mut failures: Vec<String> = Vec::new();
    for (tag, src) in programs {
        let interp = run_src(src, tag, false);
        let vm = run_src(src, tag, true);
        if let Err(diff) = compare_outputs("interpreter", &interp, "vm", &vm) {
            failures.push(format!("\n--- {tag} ---\n{diff}"));
        }
    }
    assert!(
        failures.is_empty(),
        "{} logical-operand case(s) diverged between backends:{}",
        failures.len(),
        failures.join("")
    );
}

/// RES-3896: `Array + Array` must behave identically on both backends. The
/// tree walker special-cases array concatenation in `eval_infix_expression`;
/// before the fix the VM's `Op::Add` had no arm for two arrays and raised a
/// type mismatch, so identical source produced different stdout *and* exit
/// code. Lock the class here so it can't silently return.
#[test]
fn interpreter_and_vm_agree_on_array_concatenation() {
    let programs = [
        (
            "int_arrays",
            "fn main() { let a = [1, 2]; let b = [3, 4]; println(a + b); } main();",
        ),
        (
            "string_arrays",
            "fn main() { let a = [\"x\", \"y\"]; let b = [\"z\"]; println(a + b); } main();",
        ),
        (
            "empty_left",
            "fn main() { let a: [int32] = []; let b = [1, 2]; println(a + b); } main();",
        ),
        (
            "empty_right",
            "fn main() { let a = [1, 2]; let b: [int32] = []; println(a + b); } main();",
        ),
        (
            "array_plus_int_still_errors",
            "fn main() { let a = [1, 2]; println(a + 5); } main();",
        ),
    ];
    let mut failures: Vec<String> = Vec::new();
    for (tag, src) in programs {
        let interp = run_src(src, tag, false);
        let vm = run_src(src, tag, true);
        if let Err(diff) = compare_outputs("interpreter", &interp, "vm", &vm) {
            failures.push(format!("\n--- {tag} ---\n{diff}"));
        }
    }
    assert!(
        failures.is_empty(),
        "{} array-concatenation case(s) diverged between backends:{}",
        failures.len(),
        failures.join("")
    );
}

/// RES-3902: the VM's peephole optimizer had a family of type-blind
/// numeric/bitwise identity folds (`x+0==x`, `x*0==0`, `x&0==0`,
/// `x-0==x`, `x/1==x`, ...) that assumed the operand feeding the fold
/// was always `Int`/`Float`. `+` and `*` also legally accept `String`
/// operands (concatenation/stringification, repetition — RES-924), for
/// which the assumed "identity" is a different, non-identity value; the
/// bitwise ops and `-`/`/` never accept `String` at all, so folding
/// away the op also folded away the runtime type-mismatch error it
/// would have raised. Both are silent-divergence bugs: `--vm` gives a
/// different (or no) error than the interpreter for identical source.
/// Lock the whole class here so it can't silently return.
#[test]
fn interpreter_and_vm_agree_on_typed_identity_folds() {
    let programs = [
        // `+` and `*` accept String — the peephole assumed the wrong
        // "identity" value instead of raising/suppressing an error.
        (
            "string_plus_zero",
            "fn main() { let s = \"ab\"; println(s + 0); } main();",
        ),
        (
            "string_times_zero",
            "fn main() { let s = \"ab\"; println(s * 0); } main();",
        ),
        // `-`, `/`, and the bitwise ops never accept String — the
        // peephole silently suppressed the type-mismatch error.
        (
            "string_minus_zero",
            "fn main() { let s = \"ab\"; println(s - 0); } main();",
        ),
        (
            "string_div_one",
            "fn main() { let s = \"ab\"; println(s / 1); } main();",
        ),
        (
            "string_band_zero",
            "fn main() { let s = \"ab\"; println(s & 0); } main();",
        ),
        (
            "string_bor_zero",
            "fn main() { let s = \"ab\"; println(s | 0); } main();",
        ),
        (
            "string_bxor_zero",
            "fn main() { let s = \"ab\"; println(s ^ 0); } main();",
        ),
        (
            "string_shl_zero",
            "fn main() { let s = \"ab\"; println(s << 0); } main();",
        ),
        (
            "string_shr_zero",
            "fn main() { let s = \"ab\"; println(s >> 0); } main();",
        ),
        // The legitimate numeric cases must still agree (and still
        // benefit from the fold where it's provably safe).
        (
            "int_plus_zero",
            "fn main() { let x = 5; println(x + 0); } main();",
        ),
        (
            "int_times_zero",
            "fn main() { let x = 5; println(x * 0); } main();",
        ),
        (
            "int_increment_loop",
            "fn main() { let mut x = 0; let mut i = 0; while i < 5 { x = x + 1; i = i + 1; } println(x); } main();",
        ),
    ];
    let mut failures: Vec<String> = Vec::new();
    for (tag, src) in programs {
        let interp = run_src(src, tag, false);
        let vm = run_src(src, tag, true);
        if let Err(diff) = compare_outputs("interpreter", &interp, "vm", &vm) {
            failures.push(format!("\n--- {tag} ---\n{diff}"));
        }
    }
    assert!(
        failures.is_empty(),
        "{} typed-identity-fold case(s) diverged between backends:{}",
        failures.len(),
        failures.join("")
    );
}

/// RES-3904: the compiler emits `Op::CallMethod` for every `x.y(...)`
/// dot-call uniformly (no static type info at that point), but the
/// VM's `CallMethod` handler only had an arm for `Struct`/`EnumVariant`
/// receivers — every dot-call method on a built-in `String`, `Array`,
/// `Map`, or `Set` crashed the VM with a type-mismatch, while the
/// interpreter (which has its own short-name → builtin mapping)
/// handled them fine. Lock representative methods from each container
/// type so this class can't silently return.
#[test]
fn interpreter_and_vm_agree_on_container_method_calls() {
    let programs = [
        (
            "string_to_upper",
            "fn main() { let s = \"hello\"; println(s.to_upper()); } main();",
        ),
        (
            "string_repeat",
            "fn main() { let s = \"ab\"; println(s.repeat(3)); } main();",
        ),
        (
            "string_trim",
            "fn main() { let s = \"  hi  \"; println(s.trim()); } main();",
        ),
        (
            "array_len",
            "fn main() { let a = [1, 2, 3]; println(a.len()); } main();",
        ),
        (
            "array_collect",
            "fn main() { let a = [1, 2, 3]; println(a.collect()); } main();",
        ),
        (
            "unrecognized_method_still_errors",
            "fn main() { let s = \"ab\"; println(s.not_a_real_method()); } main();",
        ),
    ];
    let mut failures: Vec<String> = Vec::new();
    for (tag, src) in programs {
        let interp = run_src(src, tag, false);
        let vm = run_src(src, tag, true);
        if let Err(diff) = compare_outputs("interpreter", &interp, "vm", &vm) {
            failures.push(format!("\n--- {tag} ---\n{diff}"));
        }
    }
    assert!(
        failures.is_empty(),
        "{} container-method case(s) diverged between backends:{}",
        failures.len(),
        failures.join("")
    );
}

/// RES-3916: bare zero-arg enum-variant references (`E::A` in expression
/// position), enum `==` / `!=` equality (including tuple/named payloads),
/// and payload-less `match` must behave identically on both backends.
/// Before the fix the VM's bytecode compiler raised `unknown identifier:
/// E::A` for bare references (they were only registered as locals in the
/// enum's *declaring* scope), and `vm_values_eq` had no `EnumVariant` arm
/// so equal variants compared unequal. (Payload-extracting `match` and
/// constructor-as-function-value remain tracked under RES-3915.)
#[test]
fn interpreter_and_vm_agree_on_enum_variant_refs_and_equality() {
    let programs = [
        (
            "bare_ref_eq",
            "fn main() { let x = E::A; if x == E::A { println(\"a\"); } else { println(\"notA\"); } } enum E { A, B } main();",
        ),
        (
            "bare_ref_neq",
            "enum E { A, B } fn main() { let x = E::A; if x != E::B { println(\"diff\"); } else { println(\"same\"); } } main();",
        ),
        (
            "bare_ref_reassign",
            "enum Color { Red, Green, Blue } fn main() { let mut c = Color::Red; c = Color::Green; if c == Color::Green { println(\"green\"); } } main();",
        ),
        (
            "no_payload_match",
            "enum E { A, B } fn main() { let x = E::B; match x { E::A => println(\"a\"), E::B => println(\"b\") } } main();",
        ),
        (
            "payload_equality",
            "enum E { P(int32, int32) } fn main() { let a = E::P(1, 2); let b = E::P(1, 2); if a == b { println(\"eq\"); } else { println(\"ne\"); } } main();",
        ),
        (
            "payload_inequality",
            "enum E { P(int32) } fn main() { let a = E::P(1); let b = E::P(2); if a == b { println(\"eq\"); } else { println(\"ne\"); } } main();",
        ),
    ];
    let mut failures: Vec<String> = Vec::new();
    for (tag, src) in programs {
        let interp = run_src(src, tag, false);
        let vm = run_src(src, tag, true);
        if let Err(diff) = compare_outputs("interpreter", &interp, "vm", &vm) {
            failures.push(format!("\n--- {tag} ---\n{diff}"));
        }
    }
    assert!(
        failures.is_empty(),
        "{} enum-variant case(s) diverged between backends:{}",
        failures.len(),
        failures.join("")
    );
}
