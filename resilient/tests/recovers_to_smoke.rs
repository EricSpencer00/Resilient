//! RES-392: integration tests for the `recovers_to` crash-recovery
//! postcondition — MVP final-state variant.
//!
//! Two cases:
//!
//!   1. `examples/recovers_to_ok.rz` — the final-state satisfies the
//!      clause, so the binary runs cleanly and prints the body's
//!      output.
//!
//!   2. `examples/recovers_to_fail.rz` — the body returns a value
//!      that falsifies the recovery invariant at runtime. The
//!      driver surfaces a `Contract violation` diagnostic that
//!      mentions `recovers_to` and includes the counterexample.
//!
//! The `.rz` extension on the failing example is intentional: the
//! broader `examples_golden` harness only walks `*.rz` files, so
//! the rejecting example won't be miscategorised as a passing
//! program whose stdout should be compared against an
//! `.expected.txt` sibling.

use std::path::PathBuf;
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rz")
}

fn examples_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples")
}

#[test]
fn recovers_to_final_state_satisfied_accepts() {
    let ex = examples_dir().join("recovers_to_ok.rz");
    assert!(ex.exists(), "missing example: {}", ex.display());

    let output = Command::new(bin())
        .arg(&ex)
        .output()
        .expect("spawn resilient binary");

    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();

    assert!(
        output.status.success(),
        "expected successful run; stdout:\n{}\nstderr:\n{}",
        stdout,
        stderr
    );
    assert!(
        stdout.contains("0"),
        "expected body output; stdout:\n{}",
        stdout
    );
    assert!(
        stdout.contains("Program executed successfully"),
        "expected clean exit marker; stdout:\n{}",
        stdout
    );
}

#[test]
fn recovers_to_final_state_violated_rejects() {
    let ex = examples_dir().join("recovers_to_fail.rz");
    assert!(ex.exists(), "missing example: {}", ex.display());

    let output = Command::new(bin())
        .arg(&ex)
        .output()
        .expect("spawn resilient binary");

    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let combined = format!("{}{}", stdout, stderr);

    assert!(
        combined.contains("Contract violation"),
        "expected Contract violation diagnostic; combined output:\n{}",
        combined
    );
    assert!(
        combined.contains("recovers_to"),
        "diagnostic must name recovers_to; combined output:\n{}",
        combined
    );
    assert!(
        combined.contains("result = 3"),
        "diagnostic must carry the final-state counterexample; \
         combined output:\n{}",
        combined
    );
}
