//! RES-3160: focused help for the `rz check` subcommand.

use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rz")
}

fn assert_focused_check_help(args: &[&str]) {
    let output = Command::new(bin())
        .args(args)
        .output()
        .expect("spawn rz check help");

    assert_eq!(
        output.status.code(),
        Some(0),
        "check help should exit 0; stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    for expected in [
        "rz check — type-check a file without running it",
        "USAGE:\n    rz check <file> [FLAGS]",
        "FLAGS:\n    -q, --quiet",
        "--emit-diagnostics-json",
        "--z3-theory MODE        Backend-limited; requires --features z3",
        "Run `rz --help` for global flags and other subcommands.",
    ] {
        assert!(
            stdout.contains(expected),
            "focused check help missing {expected:?}; got:\n{stdout}"
        );
    }
    assert!(
        !stdout.contains("COMMON FLAGS:"),
        "check help should not fall through to global help; got:\n{stdout}"
    );
}

#[test]
fn check_long_help_is_focused() {
    assert_focused_check_help(&["check", "--help"]);
}

#[test]
fn check_short_help_is_focused() {
    assert_focused_check_help(&["check", "-h"]);
}

#[test]
fn check_help_word_is_focused() {
    assert_focused_check_help(&["check", "help"]);
}
