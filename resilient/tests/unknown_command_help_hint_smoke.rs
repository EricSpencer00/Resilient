//! RES-3201: unknown command diagnostics mention the help-word route.

use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rz")
}

#[test]
fn unknown_command_hint_mentions_help_word() {
    let output = Command::new(bin())
        .arg("frobnicate")
        .output()
        .expect("spawn rz frobnicate");

    assert_eq!(
        output.status.code(),
        Some(2),
        "unknown command should remain a usage error; stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    for expected in [
        "Error: unknown command or file `frobnicate`.",
        "Run `rz help` or `rz --help` to list subcommands",
        "pass an existing file path",
    ] {
        assert!(
            stderr.contains(expected),
            "unknown command hint missing {expected:?}; got:\n{stderr}"
        );
    }
}
