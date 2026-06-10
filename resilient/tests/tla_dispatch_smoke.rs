//! RES-3181: real CLI dispatch for `rz tla ...`.

use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rz")
}

#[test]
fn tla_check_missing_file_reaches_tla_bridge() {
    let output = Command::new(bin())
        .args(["tla", "check", "/nonexistent/Spec.tla"])
        .output()
        .expect("spawn rz tla check missing file");

    assert_eq!(
        output.status.code(),
        Some(1),
        "missing TLA file should exit 1; stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Error: file not found: /nonexistent/Spec.tla"),
        "missing-file diagnostic should come from TLA bridge; stderr={stderr}"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.contains("seed="),
        "TLA dispatch should not fall through to normal execution; stdout={stdout}"
    );
}

#[test]
fn tla_top_level_help_is_focused() {
    let output = Command::new(bin())
        .args(["tla", "--help"])
        .output()
        .expect("spawn rz tla --help");

    assert_eq!(
        output.status.code(),
        Some(0),
        "tla help should exit 0; stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    for expected in [
        "rz tla — TLA+ model checking integration",
        "USAGE:\n    rz tla check [OPTIONS] <file.tla>",
        "Path to tla2tools.jar",
        "RESILIENT_TLC_JAR environment variable",
    ] {
        assert!(
            stdout.contains(expected),
            "focused TLA help missing {expected:?}; got:\n{stdout}"
        );
    }
    assert!(
        !stdout.contains("COMMON FLAGS:"),
        "tla help should not fall through to global help; got:\n{stdout}"
    );
}
