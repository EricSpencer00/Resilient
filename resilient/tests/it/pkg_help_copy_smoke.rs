//! RES-3198: package help documents help-word subcommand usage.

use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rz")
}

#[test]
fn pkg_help_documents_help_word_form() {
    let output = Command::new(bin())
        .args(["pkg", "--help"])
        .output()
        .expect("spawn rz pkg --help");

    assert_eq!(
        output.status.code(),
        Some(0),
        "pkg help should exit 0; stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    for expected in [
        "SUBCOMMANDS:",
        "init     Scaffold a new project",
        "publish",
        "add      Add a dependency",
        "rz pkg <subcommand> --help",
        "rz pkg <subcommand> help",
    ] {
        assert!(
            stdout.contains(expected),
            "pkg help missing {expected:?}; got:\n{stdout}"
        );
    }
}
