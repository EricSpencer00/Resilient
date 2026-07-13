//! RES-3189: focused help for the `rz pkg add` subcommand.

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
        "res_pkg_add_help_{}_{}_{}",
        tag,
        std::process::id(),
        n
    ));
    std::fs::create_dir_all(&p).expect("mkdir pkg add help tmp");
    p
}

fn assert_focused_pkg_add_help(args: &[&str]) {
    let output = Command::new(bin())
        .args(args)
        .output()
        .expect("spawn rz pkg add help");

    assert_eq!(
        output.status.code(),
        Some(0),
        "pkg add help should exit 0; stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    for expected in [
        "rz pkg add",
        "USAGE:\n    rz pkg add <name> path:../libs/<name>",
        "ARGS:\n    <name>",
        "--rev <r>      Pin to a git revision",
        "Validates the dependency resolves",
    ] {
        assert!(
            stdout.contains(expected),
            "focused pkg add help missing {expected:?}; got:\n{stdout}"
        );
    }
    assert!(
        !stdout.contains("COMMON FLAGS:"),
        "pkg add help should not fall through to global help; got:\n{stdout}"
    );
}

#[test]
fn pkg_add_long_help_is_focused() {
    assert_focused_pkg_add_help(&["pkg", "add", "--help"]);
}

#[test]
fn pkg_add_short_help_is_focused() {
    assert_focused_pkg_add_help(&["pkg", "add", "-h"]);
}

#[test]
fn pkg_add_help_word_is_focused() {
    assert_focused_pkg_add_help(&["pkg", "add", "help"]);
}

#[test]
fn dependency_named_help_still_uses_source_specifier() {
    let parent = tmp_parent("dependency_named_help");
    let app = parent.join("app");
    let dep = parent.join("libs/help");
    std::fs::create_dir_all(app.join("src")).expect("mkdir app src");
    std::fs::create_dir_all(dep.join("src")).expect("mkdir dep src");
    std::fs::write(
        app.join("resilient.toml"),
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n[dependencies]\n",
    )
    .expect("write app manifest");
    std::fs::write(
        dep.join("resilient.toml"),
        "[package]\nname = \"help\"\nversion = \"0.1.0\"\n",
    )
    .expect("write dep manifest");

    let output = Command::new(bin())
        .args(["pkg", "add", "help", "path:../libs/help"])
        .current_dir(&app)
        .output()
        .expect("spawn rz pkg add help path");

    assert!(
        output.status.success(),
        "expected dependency named help to be added; stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let manifest = std::fs::read_to_string(app.join("resilient.toml")).expect("read manifest");
    assert!(
        manifest.contains("help = { path = \"../libs/help\" }"),
        "dependency named help was not appended: {manifest}"
    );
    let _ = std::fs::remove_dir_all(&parent);
}
