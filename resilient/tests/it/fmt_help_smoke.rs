//! RES-3172: focused help for the `rz fmt` subcommand.

use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rz")
}

fn assert_focused_fmt_help(args: &[&str]) {
    let output = Command::new(bin())
        .args(args)
        .output()
        .expect("spawn rz fmt help");

    assert_eq!(
        output.status.code(),
        Some(0),
        "fmt help should exit 0; stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    for expected in [
        "rz fmt — format a Resilient source file",
        "USAGE:\n    rz fmt <file> [--in-place]",
        "By default, prints the formatted source to stdout.",
        "With --in-place, rewrites the file and prints nothing on success.",
        "-i, --in-place    Rewrite the file instead of printing formatted source",
        "rz fmt examples/hello.rz",
        "Run `rz --help` for global flags and other subcommands.",
    ] {
        assert!(
            stdout.contains(expected),
            "focused fmt help missing {expected:?}; got:\n{stdout}"
        );
    }
    assert!(
        !stdout.contains("COMMON FLAGS:"),
        "fmt help should not fall through to global help; got:\n{stdout}"
    );
}

#[test]
fn fmt_long_help_is_focused() {
    assert_focused_fmt_help(&["fmt", "--help"]);
}

#[test]
fn fmt_short_help_is_focused() {
    assert_focused_fmt_help(&["fmt", "-h"]);
}

#[test]
fn fmt_help_word_is_focused() {
    assert_focused_fmt_help(&["fmt", "help"]);
}
