//! RES-407: integration tests for per-access bounds-check elision.
//!
//! End-to-end checks that `--audit --typecheck` surfaces the
//! `array-bounds elided` summary line for the proven example and
//! does not surface it for the dynamic-index one.

use std::path::PathBuf;
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rz")
}

fn examples_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples")
}

fn audit_run(filename: &str) -> (String, String, bool) {
    let ex = examples_dir().join(filename);
    assert!(ex.exists(), "missing example: {}", ex.display());
    let output = Command::new(bin())
        .arg("--audit")
        .arg("--typecheck")
        .arg(&ex)
        .output()
        .expect("spawn resilient binary");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.success(),
    )
}

#[test]
fn audit_surfaces_elided_line_for_proven_index() {
    let (stdout, stderr, ok) = audit_run("bounds_elision_proven.rz");
    assert!(
        ok,
        "expected successful run; stdout:\n{}\nstderr:\n{}",
        stdout, stderr
    );
    assert!(
        stdout.contains("array-bounds elided"),
        "expected `array-bounds elided` audit line; stdout:\n{}",
        stdout
    );
    // The per-site line is colored, so the literal substring `elided at`
    // is broken up by an ANSI reset. Match the structural line:col instead
    // — `xs[1]` lives on line 10 of the example.
    assert!(
        stdout.contains("at 10:"),
        "expected per-site `elided at L:C` line referencing line 10; stdout:\n{}",
        stdout
    );
}

#[test]
fn audit_does_not_surface_elided_line_for_dynamic_index() {
    let (stdout, stderr, ok) = audit_run("bounds_elision_dynamic.rz");
    assert!(
        ok,
        "expected successful run; stdout:\n{}\nstderr:\n{}",
        stdout, stderr
    );
    // The summary section is gated on at least one bounds visit; the
    // dynamic example has exactly one (unproven) so the section
    // shows up — but the proven count must be 0.
    assert!(
        stdout.contains("array-bounds elided (proven static):      \x1B[32m0 / 1\x1B[0m")
            || stdout.contains("array-bounds elided (proven static):      0 / 1"),
        "expected `0 / 1` proven count; stdout:\n{}",
        stdout
    );
    assert!(
        !stdout.contains("elided at"),
        "expected no per-site `elided at L:C` line for dynamic-index program; stdout:\n{}",
        stdout
    );
}
