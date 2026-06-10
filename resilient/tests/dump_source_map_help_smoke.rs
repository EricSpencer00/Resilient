//! RES-3178: focused help for the `rz dump-source-map` subcommand.

use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rz")
}

fn assert_focused_dump_source_map_help(args: &[&str]) {
    let output = Command::new(bin())
        .args(args)
        .output()
        .expect("spawn rz dump-source-map help");

    assert_eq!(
        output.status.code(),
        Some(0),
        "dump-source-map help should exit 0; stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    for expected in [
        "rz dump-source-map — print bytecode-to-source-line mappings",
        "USAGE:\n    rz dump-source-map <file>",
        "Compiles the file, then prints bytecode program counters with source lines.",
        "The report includes the main chunk and one section per compiled function.",
        "rz dump-source-map examples/hello.rz",
        "Run `rz --help` for global flags and other subcommands.",
    ] {
        assert!(
            stdout.contains(expected),
            "focused dump-source-map help missing {expected:?}; got:\n{stdout}"
        );
    }
    assert!(
        !stdout.contains("COMMON FLAGS:"),
        "dump-source-map help should not fall through to global help; got:\n{stdout}"
    );
}

#[test]
fn dump_source_map_long_help_is_focused() {
    assert_focused_dump_source_map_help(&["dump-source-map", "--help"]);
}

#[test]
fn dump_source_map_short_help_is_focused() {
    assert_focused_dump_source_map_help(&["dump-source-map", "-h"]);
}

#[test]
fn dump_source_map_help_word_is_focused() {
    assert_focused_dump_source_map_help(&["dump-source-map", "help"]);
}
