//! RES-385: end-to-end smoke tests for the linear-type MVP.
//!
//! Covers both the happy-path example (single-consumer OK) and the
//! error-path example (double-consumer rejected). The happy path is
//! also exercised by the stdout golden harness; this file adds the
//! stderr-side check that the standard harness cannot express, and
//! pins the diagnostic text against
//! `examples/linear_double_use.expected.txt` so wording regressions
//! surface in CI.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_resilient")
}

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

#[test]
fn linear_demo_runs_and_prints_single_consumption() {
    let output = Command::new(bin())
        .arg("examples/linear_demo.rz")
        .current_dir(manifest_dir())
        .output()
        .expect("failed to spawn resilient");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        output.status.code(),
        Some(0),
        "linear_demo must exit 0; stdout={stdout} stderr={stderr}"
    );
    assert!(
        stdout.contains("closing fd=3"),
        "expected close() to print its fd; got:\n{stdout}"
    );
    assert!(
        stdout.contains("linear value consumed exactly once"),
        "expected main() completion line; got:\n{stdout}"
    );
}

#[test]
fn linear_demo_typechecks_clean() {
    // The happy-path example must pass `--typecheck` without triggering
    // the linear-use pass — the single consumer (`close(fh)` inside
    // `use_once`) is the whole point of the demo.
    let output = Command::new(bin())
        .arg("--typecheck")
        .arg("examples/linear_demo.rz")
        .current_dir(manifest_dir())
        .output()
        .expect("failed to spawn resilient --typecheck");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        output.status.code(),
        Some(0),
        "linear_demo --typecheck must exit 0; stderr={stderr}"
    );
    assert!(
        !stderr.contains("error[linear-use]"),
        "happy-path demo must not trigger linear-use diagnostic; got:\n{stderr}"
    );
}

#[test]
fn linear_double_use_is_rejected_with_pinned_diagnostic() {
    let output = Command::new(bin())
        .arg("--typecheck")
        .arg("examples/linear_double_use.rz")
        .current_dir(manifest_dir())
        .output()
        .expect("failed to spawn resilient --typecheck");
    assert_ne!(
        output.status.code(),
        Some(0),
        "double-consumer example must fail --typecheck"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);

    // The sibling `.expected.txt` holds the canonical diagnostic line.
    // We search for it in stderr (ANSI escapes and the caret rendering
    // mean stderr contains more than just the pinned line, but the
    // exact diagnostic shape is byte-stable).
    let expected_file = manifest_dir().join("examples/linear_double_use.expected.txt");
    let expected = fs::read_to_string(&expected_file)
        .unwrap_or_else(|e| panic!("reading {}: {}", expected_file.display(), e));
    let expected_line = expected.trim_end_matches(&['\n', '\r'][..]);
    assert!(
        stderr.contains(expected_line),
        "stderr did not contain pinned diagnostic line.\n  expected: {expected_line}\n  stderr:\n{stderr}"
    );
}
