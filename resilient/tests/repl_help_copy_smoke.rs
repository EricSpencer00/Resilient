//! RES-3205: REPL help documents the help-word usage form.

use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rz")
}

#[test]
fn repl_help_documents_help_word_usage() {
    let output = Command::new(bin())
        .args(["repl", "help"])
        .output()
        .expect("spawn rz repl help");

    assert_eq!(
        output.status.code(),
        Some(0),
        "repl help should exit 0; stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    for expected in [
        "USAGE:\n    rz repl [--examples-dir DIR]",
        "rz repl --help",
        "rz repl help",
        "FLAGS:",
        "--examples-dir DIR",
    ] {
        assert!(
            stdout.contains(expected),
            "repl help missing {expected:?}; got:\n{stdout}"
        );
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("seed="),
        "repl help copy path should not print seed banner; got:\n{stderr}"
    );
}
