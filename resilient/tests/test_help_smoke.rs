//! RES-3183: focused help for the `rz test` subcommand.

use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rz")
}

fn assert_focused_test_help(args: &[&str]) {
    let output = Command::new(bin())
        .args(args)
        .output()
        .expect("spawn rz test help");

    assert_eq!(
        output.status.code(),
        Some(0),
        "test help should exit 0; stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    for expected in [
        "Usage: rz test [<file|dir>] [--filter <substring>]",
        "Discover and run fn test_*() functions in .rz files.",
        "(no argument)       Discover from the current directory",
        "--filter <substr>   Only run tests whose name contains <substr>",
    ] {
        assert!(
            stdout.contains(expected),
            "focused test help missing {expected:?}; got:\n{stdout}"
        );
    }
    assert!(
        !stdout.contains("COMMON FLAGS:"),
        "test help should not fall through to global help; got:\n{stdout}"
    );
}

#[test]
fn test_long_help_is_focused() {
    assert_focused_test_help(&["test", "--help"]);
}

#[test]
fn test_short_help_is_focused() {
    assert_focused_test_help(&["test", "-h"]);
}

#[test]
fn test_help_word_is_focused() {
    assert_focused_test_help(&["test", "help"]);
}
