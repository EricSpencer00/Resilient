//! RES-3194: focused REPL help should not print the runtime seed banner.

use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rz")
}

fn assert_repl_help_without_seed(args: &[&str]) {
    let output = Command::new(bin())
        .args(args)
        .output()
        .expect("spawn rz repl help");

    assert_eq!(
        output.status.code(),
        Some(0),
        "repl help should exit 0; stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    for expected in [
        "rz repl",
        "USAGE:\n    rz repl [--examples-dir DIR]",
        "FLAGS:\n    --help, -h",
        "For bare REPL startup, run plain `rz`.",
    ] {
        assert!(
            stdout.contains(expected),
            "focused repl help missing {expected:?}; got:\n{stdout}"
        );
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("seed="),
        "repl help should not print seed banner; got stderr:\n{stderr}"
    );
}

#[test]
fn repl_help_word_has_no_seed_banner() {
    assert_repl_help_without_seed(&["repl", "help"]);
}

#[test]
fn repl_long_help_has_no_seed_banner() {
    assert_repl_help_without_seed(&["repl", "--help"]);
}

#[test]
fn repl_short_help_has_no_seed_banner() {
    assert_repl_help_without_seed(&["repl", "-h"]);
}
