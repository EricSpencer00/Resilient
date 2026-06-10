//! RES-3190: focused help for the `rz pkg init` subcommand.

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
        "res_pkg_init_help_{}_{}_{}",
        tag,
        std::process::id(),
        n
    ));
    std::fs::create_dir_all(&p).expect("mkdir pkg init help tmp");
    p
}

fn assert_focused_pkg_init_help(args: &[&str]) {
    let output = Command::new(bin())
        .args(args)
        .output()
        .expect("spawn rz pkg init help");

    assert_eq!(
        output.status.code(),
        Some(0),
        "pkg init help should exit 0; stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    for expected in [
        "rz pkg init",
        "scaffold a new project",
        "USAGE:\n    rz pkg init <name>",
        "rz pkg init --name <n>",
        "Refuses to overwrite an existing manifest",
    ] {
        assert!(
            stdout.contains(expected),
            "focused pkg init help missing {expected:?}; got:\n{stdout}"
        );
    }
    assert!(
        !stdout.contains("COMMON FLAGS:"),
        "pkg init help should not fall through to global help; got:\n{stdout}"
    );
}

#[test]
fn pkg_init_long_help_is_focused() {
    assert_focused_pkg_init_help(&["pkg", "init", "--help"]);
}

#[test]
fn pkg_init_short_help_is_focused() {
    assert_focused_pkg_init_help(&["pkg", "init", "-h"]);
}

#[test]
fn pkg_init_help_word_is_focused() {
    assert_focused_pkg_init_help(&["pkg", "init", "help"]);
}

#[test]
fn name_flag_can_still_create_project_named_help() {
    let parent = tmp_parent("name_help");
    let output = Command::new(bin())
        .args(["pkg", "init", "--name", "help"])
        .current_dir(&parent)
        .output()
        .expect("spawn rz pkg init --name help");

    assert!(
        output.status.success(),
        "expected --name help to scaffold a project; stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let manifest =
        std::fs::read_to_string(parent.join("help/resilient.toml")).expect("read manifest");
    assert!(
        manifest.contains("name = \"help\""),
        "expected project named help in manifest: {manifest}"
    );
    let _ = std::fs::remove_dir_all(&parent);
}
