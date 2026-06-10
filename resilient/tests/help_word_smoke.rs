//! RES-3193: top-level `rz help` routes to global usage.

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rz")
}

fn tmp_parent(tag: &str) -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let p = std::env::temp_dir().join(format!(
        "res_help_word_{}_{}_{}",
        tag,
        std::process::id(),
        n
    ));
    std::fs::create_dir_all(&p).expect("mkdir help word tmp");
    p
}

#[test]
fn help_word_prints_global_usage() {
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
        "COMMON FLAGS:",
        "-h, --help",
        "SUBCOMMANDS:",
        "See SYNTAX.md for the language reference.",
    ] {
        assert!(
            stdout.contains(expected),
            "global help missing {expected:?}; got:\n{stdout}"
        );
    }
}

#[test]
fn file_named_help_still_runs_as_a_file() {
    let parent = tmp_parent("file_named_help");
    let help_path = parent.join("help");
    std::fs::write(&help_path, "println(\"file-help\");\n").expect("write help source");

    let output = Command::new(bin())
        .arg("help")
        .current_dir(&parent)
        .output()
        .expect("spawn rz help file");

    assert!(
        output.status.success(),
        "expected file named help to run; stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("file-help"),
        "expected file output, got:\n{stdout}"
    );
    assert!(
        !stdout.contains("COMMON FLAGS:"),
        "file named help should not print global help; got:\n{stdout}"
    );
    let _ = std::fs::remove_dir_all(&parent);
}
