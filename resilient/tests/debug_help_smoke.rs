//! RES-3171: focused help for the `rz debug` subcommand.

use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rz")
}

fn assert_focused_debug_help(args: &[&str]) {
    let output = Command::new(bin())
        .args(args)
        .output()
        .expect("spawn rz debug help");

    assert_eq!(
        output.status.code(),
        Some(0),
        "debug help should exit 0; stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    for expected in [
        "rz debug — start the Debug Adapter Protocol server",
        "USAGE:\n    rz debug <file>",
        "Starts a DAP server on stdin/stdout for an editor or debugger client.",
        "The file argument labels the session; the DAP launch request supplies the program path.",
        "rz debug examples/hello.rz",
        "For direct adapter launches, clients may use `rz --dap`.",
        "Run `rz --help` for global flags and other subcommands.",
    ] {
        assert!(
            stdout.contains(expected),
            "focused debug help missing {expected:?}; got:\n{stdout}"
        );
    }
    assert!(
        !stdout.contains("COMMON FLAGS:"),
        "debug help should not fall through to global help; got:\n{stdout}"
    );
    assert!(
        !stderr.contains("Starting DAP server"),
        "debug help should not start the DAP server; stderr={stderr}"
    );
}

#[test]
fn debug_long_help_is_focused() {
    assert_focused_debug_help(&["debug", "--help"]);
}

#[test]
fn debug_short_help_is_focused() {
    assert_focused_debug_help(&["debug", "-h"]);
}

#[test]
fn debug_help_word_is_focused() {
    assert_focused_debug_help(&["debug", "help"]);
}
