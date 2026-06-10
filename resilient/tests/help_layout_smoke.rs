//! RES-3155: pin readable indentation in CLI help output.

use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rz")
}

#[test]
fn global_help_preserves_nested_indentation() {
    let output = Command::new(bin())
        .arg("--help")
        .output()
        .expect("spawn rz --help");

    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    for expected in [
        "USAGE:\n    rz [FLAGS] [<file>]\n    rz                      # start REPL",
        "COMMON FLAGS:\n    -h, --help                   Show this help and exit",
        "    -t, --typecheck              Run the static type checker in strict mode\n                                 (fail with exit 1 on any type error).",
        "STATUS:\n    stable             Supported for scripts and CI on the default build",
        "    backend-limited    Stable when the named backend/build feature is present;\n                       unavailable builds print a rebuild hint",
        "SUBCOMMANDS:\n    repl                 Start interactive REPL (alias for bare `rz`)",
        "    verify-all <dir>     Re-check every obligation in a manifest\n                        (backend-limited; requires --features z3)",
    ] {
        assert!(
            stdout.contains(expected),
            "global help missing formatted block {expected:?}; got:\n{stdout}"
        );
    }
}

#[test]
fn repl_help_preserves_flags_and_examples_layout() {
    let output = Command::new(bin())
        .args(["repl", "--help"])
        .output()
        .expect("spawn rz repl --help");

    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    for expected in [
        "USAGE:\n    rz repl [--examples-dir DIR]\n    rz repl --help",
        "FLAGS:\n    --help, -h            Show this help and exit\n    --examples-dir DIR    REPL examples directory",
        "EXAMPLES:\n    rz repl                   # start REPL\n    rz repl --examples-dir .  # use the current directory for `examples`",
        "For bare REPL startup, run plain `rz`.",
    ] {
        assert!(
            stdout.contains(expected),
            "repl help missing formatted block {expected:?}; got:\n{stdout}"
        );
    }
}
