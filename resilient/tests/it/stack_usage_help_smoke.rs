//! RES-3166: focused help for the `rz stack-usage` subcommand.

use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rz")
}

fn assert_focused_stack_usage_help(args: &[&str]) {
    let output = Command::new(bin())
        .args(args)
        .output()
        .expect("spawn rz stack-usage help");

    assert_eq!(
        output.status.code(),
        Some(0),
        "stack-usage help should exit 0; stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    for expected in [
        "rz stack-usage — estimate per-function worst-case stack use",
        "USAGE:\n    rz stack-usage <file>",
        "Prints one row per user function with estimated bytes and notes.",
        "Recursive call chains are reported as unbounded.",
        "#[stack(bytes=N)] declarations are checked against estimates.",
        "The command exits 1 when any declared budget is exceeded.",
        "rz stack-usage examples/stack_budget.rz",
        "Run `rz --help` for global flags and other subcommands.",
    ] {
        assert!(
            stdout.contains(expected),
            "focused stack-usage help missing {expected:?}; got:\n{stdout}"
        );
    }
    assert!(
        !stdout.contains("COMMON FLAGS:"),
        "stack-usage help should not fall through to global help; got:\n{stdout}"
    );
}

#[test]
fn stack_usage_long_help_is_focused() {
    assert_focused_stack_usage_help(&["stack-usage", "--help"]);
}

#[test]
fn stack_usage_short_help_is_focused() {
    assert_focused_stack_usage_help(&["stack-usage", "-h"]);
}

#[test]
fn stack_usage_help_word_is_focused() {
    assert_focused_stack_usage_help(&["stack-usage", "help"]);
}
