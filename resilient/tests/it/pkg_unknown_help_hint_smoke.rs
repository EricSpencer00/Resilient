//! RES-3202: unknown package subcommands mention the help-word route.

use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rz")
}

#[test]
fn unknown_pkg_subcommand_hint_mentions_pkg_help_word() {
    let output = Command::new(bin())
        .args(["pkg", "floofnicate"])
        .output()
        .expect("spawn rz pkg floofnicate");

    assert_eq!(
        output.status.code(),
        Some(2),
        "unknown pkg subcommand should remain a usage error; stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    for expected in [
        "Error: unknown pkg subcommand `floofnicate`.",
        "Known: init, publish, add.",
        "Run `rz pkg help` or `rz pkg --help` for usage.",
    ] {
        assert!(
            stderr.contains(expected),
            "unknown pkg hint missing {expected:?}; got:\n{stderr}"
        );
    }
}
