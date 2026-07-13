//! RES-3173: focused help for the `rz self-host-parity-report` subcommand.

use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rz")
}

fn assert_focused_self_host_parity_help(args: &[&str]) {
    let output = Command::new(bin())
        .args(args)
        .output()
        .expect("spawn rz self-host-parity-report help");

    assert_eq!(
        output.status.code(),
        Some(0),
        "self-host parity help should exit 0; stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    for expected in [
        "rz self-host-parity-report — publish self-hosting parity coverage",
        "USAGE:\n    rz self-host-parity-report [DIR] [--json-out PATH]",
        "DIR defaults to self-host/parity_corpus.",
        "The report compares Rust and self-host lexer/parser output for that corpus.",
        "Prints a coverage and divergence summary to stdout.",
        "With --json-out, also writes a stable JSON report artifact.",
        "--json-out PATH    Write the machine-readable report to PATH",
        "rz self-host-parity-report self-host/parity_corpus --json-out parity.json",
        "Run `rz --help` for global flags and other subcommands.",
    ] {
        assert!(
            stdout.contains(expected),
            "focused self-host parity help missing {expected:?}; got:\n{stdout}"
        );
    }
    assert!(
        !stdout.contains("COMMON FLAGS:"),
        "self-host parity help should not fall through to global help; got:\n{stdout}"
    );
}

#[test]
fn self_host_parity_long_help_is_focused() {
    assert_focused_self_host_parity_help(&["self-host-parity-report", "--help"]);
}

#[test]
fn self_host_parity_short_help_is_focused() {
    assert_focused_self_host_parity_help(&["self-host-parity-report", "-h"]);
}

#[test]
fn self_host_parity_help_word_is_focused() {
    assert_focused_self_host_parity_help(&["self-host-parity-report", "help"]);
}
