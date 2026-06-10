//! RES-3197: global help documents the top-level `rz help` word.

use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rz")
}

#[test]
fn global_help_documents_help_word_usage() {
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
        "USAGE:\n    rz [FLAGS] [<file>]",
        "rz help                 # show this help",
        "COMMON FLAGS:",
        "SUBCOMMANDS:",
    ] {
        assert!(
            stdout.contains(expected),
            "global help missing {expected:?}; got:\n{stdout}"
        );
    }
}
