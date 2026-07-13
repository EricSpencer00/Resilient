//! RES-3177: focused help for the `rz tla check` subcommand.

use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rz")
}

fn assert_focused_tla_check_help(args: &[&str]) {
    let output = Command::new(bin())
        .args(args)
        .output()
        .expect("spawn rz tla check help");

    assert_eq!(
        output.status.code(),
        Some(0),
        "tla check help should exit 0; stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    for expected in [
        "rz tla check — run TLC model checking for a TLA+ spec",
        "USAGE:\n    rz tla check [OPTIONS] <file.tla>",
        "Shells out to TLC via `java -jar tla2tools.jar`.",
        "TLC output is surfaced as Resilient-style diagnostics.",
        "--tlc-jar PATH   Path to tla2tools.jar; overrides RESILIENT_TLC_JAR",
        "--verbose        Print raw TLC output before diagnostics",
        "RESILIENT_TLC_JAR environment variable",
        "rz tla check --tlc-jar /opt/tla2tools.jar --verbose MySpec.tla",
        "Run `rz --help` for global flags and other subcommands.",
    ] {
        assert!(
            stdout.contains(expected),
            "focused tla check help missing {expected:?}; got:\n{stdout}"
        );
    }
    assert!(
        !stdout.contains("COMMON FLAGS:"),
        "tla check help should not fall through to global help; got:\n{stdout}"
    );
}

#[test]
fn tla_check_long_help_is_focused() {
    assert_focused_tla_check_help(&["tla", "check", "--help"]);
}

#[test]
fn tla_check_short_help_is_focused() {
    assert_focused_tla_check_help(&["tla", "check", "-h"]);
}

#[test]
fn tla_check_help_word_is_focused() {
    assert_focused_tla_check_help(&["tla", "check", "help"]);
}
