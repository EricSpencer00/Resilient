//! RES-4218: end-to-end coverage for body-aware `ensures` refutation.
//!
//! The unit tests in `src/ensures_refutation.rs` drive the pass
//! directly; this file drives the real `rz` binary so the wiring
//! through `typechecker.rs`'s `<EXTENSION_PASSES>` block, the
//! diagnostic rendering, and the process exit code are all covered.
//!
//! Gated on `feature = "z3"`: without the SMT backend the prover
//! degrades to `Unknown`, no clause can reach the refuted state, and
//! every program here compiles exactly as it did before RES-4218. That
//! is the intended no-z3 behaviour, not a gap — the `no_z3_*` test at
//! the bottom of `src/ensures_refutation.rs` covers the degradation.
#![cfg(feature = "z3")]

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rz")
}

/// Write `src` to a uniquely-named scratch file and return its path.
fn scratch(tag: &str, src: &str) -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("res_4218_{}_{}_{}", tag, std::process::id(), n));
    std::fs::create_dir_all(&dir).expect("mkdir scratch");
    let p = dir.join("case.rz");
    std::fs::write(&p, src).expect("write scratch source");
    p
}

fn typecheck(src: &str, tag: &str) -> (bool, String) {
    let path = scratch(tag, src);
    let out = Command::new(bin())
        .args(["--typecheck", "--seed", "0"])
        .arg(&path)
        .output()
        .expect("spawn rz");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (out.status.success(), combined)
}

/// The ticket's repro: `broken_max` returns `x` unconditionally, so
/// `ensures result >= y` is false whenever `y > x`. Before RES-4218
/// this printed `Type check passed` and only failed at runtime.
#[test]
fn broken_max_is_rejected_at_compile_time() {
    let (ok, out) = typecheck(
        "fn broken_max(int x, int y) -> int\n\
             ensures result >= x\n\
             ensures result >= y\n\
         {\n\
             return x;\n\
         }\n\
         fn main() { println(broken_max(1, 5)); }\n\
         main();\n",
        "broken_max",
    );
    assert!(!ok, "a refuted postcondition must fail the build:\n{out}");
    assert!(
        out.contains("violates `ensures result >= y`"),
        "diagnostic must name the refuted clause:\n{out}"
    );
    assert!(
        out.contains("counterexample:"),
        "a refutation must carry the falsifying assignment:\n{out}"
    );
    // The program must not have run — the whole point is that the
    // violation is caught before the runtime check fires.
    assert!(
        !out.contains("Program executed successfully"),
        "rejected program must not execute:\n{out}"
    );
}

/// The clause `broken_max` *does* satisfy (`result >= x` for
/// `return x;`) must not be reported. A pass that flagged the whole
/// function rather than the specific clause would trip this.
#[test]
fn only_the_refuted_clause_is_reported() {
    let (_ok, out) = typecheck(
        "fn broken_max(int x, int y) -> int\n\
             ensures result >= x\n\
             ensures result >= y\n\
         { return x; }\n",
        "one_clause",
    );
    assert!(
        !out.contains("violates `ensures result >= x`"),
        "`result >= x` holds for `return x;` and must not be flagged:\n{out}"
    );
}

/// The correct `max` compiles — and, because the case split discharges
/// both clauses against the body, without the `partial-proof` warnings
/// that every `result`-constrained clause used to emit.
#[test]
fn verified_max_example_compiles_without_partial_proof_warnings() {
    let ex = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("examples")
        .join("verified_max.rz");
    assert!(ex.exists(), "missing example: {}", ex.display());
    let out = Command::new(bin())
        .args(["--typecheck", "--seed", "0"])
        .arg(&ex)
        .output()
        .expect("spawn rz");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        out.status.success(),
        "a correct max must type-check:\n{combined}"
    );
    assert!(
        !combined.contains("partial-proof"),
        "both clauses are proven against the body — no partial proof remains:\n{combined}"
    );
}

/// Preconditions are assumed, so a body that only satisfies the
/// postcondition under its `requires` still compiles.
#[test]
fn preconditions_discharge_the_obligation() {
    let (ok, out) = typecheck(
        "fn clamped(int x, int y) -> int\n\
             requires x >= y\n\
             ensures result >= y\n\
         { return x; }\n",
        "requires",
    );
    assert!(
        ok,
        "`requires x >= y` makes `return x;` satisfy `result >= y`:\n{out}"
    );
}

/// A body outside the modelled subset keeps the old behaviour: the
/// clause is not grounded, so nothing is refuted and the runtime check
/// stays in place. This is the guardrail against false positives.
#[test]
fn out_of_subset_body_still_compiles() {
    let (ok, out) = typecheck(
        "fn helper(int x) -> int { return x; }\n\
         fn wrapped(int x, int y) -> int\n\
             ensures result >= y\n\
         { return helper(x); }\n",
        "out_of_subset",
    );
    assert!(
        ok,
        "a call-returning body is outside the modelled subset and must not be refuted:\n{out}"
    );
}
