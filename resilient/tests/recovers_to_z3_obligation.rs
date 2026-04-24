//! RES-222: Z3 proof obligation for `recovers_to` invariants.
//!
//! RES-387 added the parser + typechecker infrastructure for `fails`
//! and `recovers_to`, but the verifier ignored the recovery clause.
//! This ticket wires `recovers_to` into the existing Z3 discharge
//! pipeline: every `recovers_to: EXPR;` on a fn with a non-empty
//! `fails` set is a real proof obligation. With no handler (structured
//! try/catch is a separate ticket) Z3 must prove the invariant holds
//! under the declared `requires`; an undecidable verdict is a compile
//! error, a timeout degrades to a hint (mirrors requires/ensures).
//!
//! The tests here invoke the driver with `--typecheck` so the Z3
//! discharge path runs. The plain `cargo run` path skips typechecking
//! and the runtime contract check still fires — that older shape is
//! covered by `recovers_to_smoke.rs`.
//!
//! Only compiled when the `z3` feature is enabled; without Z3, the
//! shim returns `None` and no obligation fires.

#![cfg(feature = "z3")]

use std::path::PathBuf;
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_resilient")
}

fn examples_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples")
}

fn typecheck(example_file: &str) -> std::process::Output {
    let ex = examples_dir().join(example_file);
    assert!(ex.exists(), "missing example: {}", ex.display());
    Command::new(bin())
        .arg("--typecheck")
        .arg(&ex)
        .output()
        .expect("spawn resilient binary")
}

#[test]
fn recovers_to_handled_ok_typechecks_cleanly() {
    // fn declares `fails` and a `recovers_to` clause provable from
    // requires — the verifier discharges it and the type checker
    // reports success.
    let output = typecheck("recovers_to_handled_ok.rz");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let combined = format!("{}{}", stdout, stderr);
    assert!(
        output.status.success(),
        "expected typecheck to pass; combined output:\n{}",
        combined
    );
    assert!(
        combined.contains("Type check passed"),
        "expected `Type check passed` banner; got:\n{}",
        combined
    );
    assert!(
        !combined.contains("recovers_to invariant cannot be proven"),
        "no obligation failure expected on the handled path; got:\n{}",
        combined
    );
}

#[test]
fn recovers_to_unhandled_safe_typechecks_cleanly() {
    // `fails` set non-empty, no handler, but the recovery invariant
    // is entailed by the requires precondition — Z3 proves it and
    // the typechecker accepts the program.
    let output = typecheck("recovers_to_unhandled_safe.rz");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let combined = format!("{}{}", stdout, stderr);
    assert!(
        output.status.success(),
        "expected typecheck to pass on the non-destructive path; \
         combined output:\n{}",
        combined
    );
    assert!(
        combined.contains("Type check passed"),
        "expected `Type check passed` banner; got:\n{}",
        combined
    );
}

#[test]
fn recovers_to_unhandled_destructive_is_rejected() {
    // `fails` non-empty, no handler, and the recovery invariant is
    // NOT entailed by the requires — Z3 cannot prove it, so the
    // typechecker rejects the program.
    let output = typecheck("recovers_to_unhandled_destructive.res");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let combined = format!("{}{}", stdout, stderr);

    assert!(
        !output.status.success(),
        "expected typecheck to reject the destructive path; \
         combined output:\n{}",
        combined
    );
    assert!(
        combined.contains("recovers_to"),
        "diagnostic must name `recovers_to`; combined output:\n{}",
        combined
    );
    assert!(
        combined.contains("cannot be proven"),
        "diagnostic must say `cannot be proven`; combined output:\n{}",
        combined
    );
    // The diagnostic must point at the `recovers_to:` clause's source
    // position (line 20 in the fixture, where the clause begins).
    assert!(
        combined.contains("20:"),
        "diagnostic must carry the clause's line:col; combined output:\n{}",
        combined
    );
    // And should echo the fails set so the user knows which variants
    // drove the obligation.
    assert!(
        combined.contains("Timeout"),
        "diagnostic must mention the declared fails variant; \
         combined output:\n{}",
        combined
    );
}
