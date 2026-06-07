use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rz")
}

#[test]
fn repl_alias_shows_help() {
    // `rz repl --help` should route to the dedicated REPL help
    // path and exit success.
    let output = Command::new(bin())
        .args(["repl", "--help"])
        .output()
        .expect("spawn resilient repl --help");

    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("rz repl"),
        "repl help output missing heading: {stdout}"
    );
    assert!(
        stdout.contains("--examples-dir"),
        "repl help output missing examples dir flag: {stdout}"
    );
}

#[test]
fn help_includes_repl_subcommand() {
    // Discoverability check for the explicit alias.
    let output = Command::new(bin())
        .arg("--help")
        .output()
        .expect("spawn resilient --help");

    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("repl"),
        "global help output missing repl subcommand: {stdout}"
    );
}
