//! RES-3206: global help documents focused subcommand help forms.

use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rz")
}

#[test]
fn global_help_mentions_focused_subcommand_help_forms() {
    let output = Command::new(bin())
        .arg("help")
        .output()
        .expect("spawn rz help");

    assert_eq!(
        output.status.code(),
        Some(0),
        "rz help should exit 0; stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    for expected in [
        "SUBCOMMANDS:",
        "pkg <verb>",
        "rz <subcommand> --help",
        "rz <subcommand> help",
        "focused usage",
    ] {
        assert!(
            stdout.contains(expected),
            "global help missing {expected:?}; got:\n{stdout}"
        );
    }
}
