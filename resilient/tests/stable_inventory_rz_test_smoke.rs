//! RES-3301: stable inventory tracks the documented `rz test` surface.

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rz")
}

fn temp_workspace() -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "resilient_rz_test_smoke_{}_{}",
        std::process::id(),
        unique
    ));
    fs::create_dir_all(&dir).expect("create temp rz test workspace");
    dir
}

#[test]
fn stable_inventory_promotes_documented_rz_test_surface() {
    let inventory = include_str!("../../docs/stable-regression-inventory.md");
    let tooling = include_str!("../../docs/tooling.md");

    assert!(
        tooling.contains("### `rz test [<file|dir>] [--filter <substring>]`"),
        "tooling docs should describe the current rz test command"
    );
    assert!(
        inventory.contains(
            "| `rz test [<file|dir>]` | `resilient/tests/stable_inventory_rz_test_smoke.rs`, `resilient/tests/test_help_smoke.rs` | Covered | Direct runner execution and focused help smoke. |"
        ),
        "stable inventory should count rz test as covered stable CLI surface"
    );
    assert!(
        !inventory.contains(
            "docs/tooling.md` still classifies the first-class test runner as future work"
        ),
        "stable inventory should not retain stale future-work wording for rz test"
    );
}

#[test]
fn rz_test_runs_discovered_tests_and_filter() {
    let dir = temp_workspace();
    let sample = dir.join("sample.rz");
    fs::write(
        &sample,
        r#"use std::testing;

fn test_addition() {
    testing_assert_eq(1 + 1, 2);
}

fn test_string() {
    testing_assert_eq("ok", "ok");
}

fn helper() {
    println("not a test");
}
"#,
    )
    .expect("write rz test sample");

    let output = Command::new(bin())
        .arg("test")
        .arg(&sample)
        .output()
        .expect("run rz test sample");
    assert_eq!(
        output.status.code(),
        Some(0),
        "rz test should pass; stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    for expected in [
        "test test_addition ... ok",
        "test test_string ... ok",
        "2 tests: 2 passed, 0 failed",
    ] {
        assert!(
            stdout.contains(expected),
            "rz test output missing {expected:?}; got:\n{stdout}"
        );
    }

    let filtered = Command::new(bin())
        .arg("test")
        .arg(&sample)
        .arg("--filter")
        .arg("addition")
        .output()
        .expect("run filtered rz test sample");
    assert_eq!(
        filtered.status.code(),
        Some(0),
        "filtered rz test should pass; stdout={} stderr={}",
        String::from_utf8_lossy(&filtered.stdout),
        String::from_utf8_lossy(&filtered.stderr)
    );
    let filtered_stdout = String::from_utf8_lossy(&filtered.stdout);
    assert!(
        filtered_stdout.contains("test test_addition ... ok"),
        "filtered rz test should run matching test; got:\n{filtered_stdout}"
    );
    assert!(
        !filtered_stdout.contains("test test_string ... ok"),
        "filtered rz test should skip non-matching test; got:\n{filtered_stdout}"
    );
    assert!(
        filtered_stdout.contains("1 test: 1 passed, 0 failed"),
        "filtered rz test should summarize one passing test; got:\n{filtered_stdout}"
    );

    let _ = fs::remove_dir_all(&dir);
}
