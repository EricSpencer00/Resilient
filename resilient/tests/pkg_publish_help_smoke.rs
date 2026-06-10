//! RES-3185: focused help for the `rz pkg publish` subcommand.

use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rz")
}

fn assert_focused_pkg_publish_help(args: &[&str]) {
    let output = Command::new(bin())
        .args(args)
        .output()
        .expect("spawn rz pkg publish help");

    assert_eq!(
        output.status.code(),
        Some(0),
        "pkg publish help should exit 0; stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    for expected in [
        "package the current project for upload to a registry",
        "USAGE:\n    rz pkg publish --dry-run",
        "--dry-run   Required today",
        "AUTH:",
        "WHAT IT DOES:",
    ] {
        assert!(
            stdout.contains(expected),
            "focused pkg publish help missing {expected:?}; got:\n{stdout}"
        );
    }
    assert!(
        !stdout.contains("COMMON FLAGS:"),
        "pkg publish help should not fall through to global help; got:\n{stdout}"
    );
}

#[test]
fn pkg_publish_long_help_is_focused() {
    assert_focused_pkg_publish_help(&["pkg", "publish", "--help"]);
}

#[test]
fn pkg_publish_short_help_is_focused() {
    assert_focused_pkg_publish_help(&["pkg", "publish", "-h"]);
}

#[test]
fn pkg_publish_help_word_is_focused() {
    assert_focused_pkg_publish_help(&["pkg", "publish", "help"]);
}
