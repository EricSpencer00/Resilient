//! RES-3165: focused help for the `rz lint` subcommand.

use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rz")
}

fn assert_focused_lint_help(args: &[&str]) {
    let output = Command::new(bin())
        .args(args)
        .output()
        .expect("spawn rz lint help");

    assert_eq!(
        output.status.code(),
        Some(0),
        "lint help should exit 0; stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    for expected in [
        "rz lint — run Resilient lints without executing the file",
        "USAGE:\n    rz lint <file> [FLAGS]",
        "rz lint --explain LCODE",
        "--deny LCODE            Promote the named lint to error severity",
        "--allow LCODE           Suppress the named lint",
        "--explain LCODE         Print the lint explanation and exit",
        "--emit-diagnostics-json Emit lint diagnostics as JSON",
        "--safety-critical       Promote safety-critical lint failures",
        "Run `rz --help` for global flags and other subcommands.",
    ] {
        assert!(
            stdout.contains(expected),
            "focused lint help missing {expected:?}; got:\n{stdout}"
        );
    }
    assert!(
        !stdout.contains("COMMON FLAGS:"),
        "lint help should not fall through to global help; got:\n{stdout}"
    );
}

#[test]
fn lint_long_help_is_focused() {
    assert_focused_lint_help(&["lint", "--help"]);
}

#[test]
fn lint_short_help_is_focused() {
    assert_focused_lint_help(&["lint", "-h"]);
}

#[test]
fn lint_help_word_is_focused() {
    assert_focused_lint_help(&["lint", "help"]);
}
