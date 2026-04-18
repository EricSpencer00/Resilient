//! RES-205: integration test for the `resilient pkg init <name>`
//! subcommand. Spawns the real binary inside a scratch parent
//! directory, asserts the expected file layout, and checks that the
//! manifest + gitignore have the templated content.
//!
//! Unit tests in `src/pkg_init.rs` cover the scaffolding logic
//! itself (empty-dir accept, non-empty-dir refuse, template byte-
//! equality, etc.) without going through the CLI. This smoke test
//! pins the CLI wiring: arg parsing, cwd handling, exit code.

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_resilient")
}

fn tmp_parent(tag: &str) -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let p = std::env::temp_dir().join(format!(
        "res_pkg_smoke_{}_{}_{}",
        tag,
        std::process::id(),
        n
    ));
    std::fs::create_dir_all(&p).expect("mkdir smoke tmp");
    p
}

#[test]
fn pkg_init_creates_project_skeleton() {
    let parent = tmp_parent("create");
    let output = Command::new(bin())
        .args(["pkg", "init", "hello_res"])
        .current_dir(&parent)
        .output()
        .expect("spawn resilient pkg init");

    assert!(
        output.status.success(),
        "expected exit 0, got {:?}\nstdout: {}\nstderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );

    // Expected layout.
    let root = parent.join("hello_res");
    assert!(root.is_dir(), "project root {} missing", root.display());
    assert!(root.join("Resilient.toml").is_file(), "manifest missing");
    assert!(root.join("src/main.rs").is_file(), "entry point missing");
    assert!(root.join(".gitignore").is_file(), "gitignore missing");

    // Manifest content — pin [package] + name to catch template
    // drift.
    let manifest = std::fs::read_to_string(root.join("Resilient.toml"))
        .expect("read manifest");
    assert!(manifest.contains("[package]"), "missing [package]: {manifest}");
    assert!(
        manifest.contains(r#"name = "hello_res""#),
        "missing name in: {manifest}"
    );
    assert!(
        manifest.contains(r#"version = "0.1.0""#),
        "missing version in: {manifest}"
    );
    assert!(
        manifest.contains("edition = "),
        "missing edition in: {manifest}"
    );

    // Entry point has the hello-world greeting.
    let main_src = std::fs::read_to_string(root.join("src/main.rs"))
        .expect("read main.rs");
    assert!(
        main_src.contains("Hello, world!"),
        "missing greeting in: {main_src}"
    );

    // Gitignore ignores target/ and cert/.
    let gi = std::fs::read_to_string(root.join(".gitignore"))
        .expect("read gitignore");
    assert!(gi.contains("target/"), "missing target/ in: {gi}");
    assert!(gi.contains("cert/"), "missing cert/ in: {gi}");

    let _ = std::fs::remove_dir_all(&parent);
}

#[test]
fn pkg_init_errors_on_nonempty_directory() {
    // Pre-seed the target dir with a stray file — the scaffolder
    // must refuse, emit a helpful error, and NOT touch the stray.
    let parent = tmp_parent("nonempty");
    let target = parent.join("occupied");
    std::fs::create_dir(&target).unwrap();
    std::fs::write(target.join("keepme.txt"), "do not disturb").unwrap();

    let output = Command::new(bin())
        .args(["pkg", "init", "occupied"])
        .current_dir(&parent)
        .output()
        .expect("spawn resilient pkg init");

    assert!(
        !output.status.success(),
        "expected non-zero exit, got success. stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.to_lowercase().contains("non-empty") || stderr.to_lowercase().contains("not empty"),
        "expected non-empty-directory diagnostic, got stderr: {stderr}"
    );

    // Pre-existing content survived.
    assert_eq!(
        std::fs::read_to_string(target.join("keepme.txt")).unwrap(),
        "do not disturb",
    );
    // And no manifest was written.
    assert!(
        !target.join("Resilient.toml").exists(),
        "manifest leaked into non-empty dir"
    );

    let _ = std::fs::remove_dir_all(&parent);
}

#[test]
fn pkg_init_missing_name_errors() {
    // Bare `resilient pkg init` without a name should exit non-zero
    // with a helpful usage hint.
    let parent = tmp_parent("missing_name");
    let output = Command::new(bin())
        .args(["pkg", "init"])
        .current_dir(&parent)
        .output()
        .expect("spawn resilient pkg init");

    assert!(!output.status.success(), "expected non-zero exit");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("requires a project name")
            || stderr.contains("<name>"),
        "expected usage hint, got: {stderr}"
    );
    let _ = std::fs::remove_dir_all(&parent);
}

#[test]
fn pkg_unknown_subcommand_errors() {
    // `resilient pkg whatever` should bail gracefully with a
    // "known subcommands" hint rather than hitting the compiler
    // driver and looking for a file named `pkg`.
    let parent = tmp_parent("unknown_sub");
    let output = Command::new(bin())
        .args(["pkg", "floofnicate"])
        .current_dir(&parent)
        .output()
        .expect("spawn resilient pkg floofnicate");

    assert!(!output.status.success(), "expected non-zero exit");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("unknown pkg subcommand") || stderr.contains("floofnicate"),
        "expected error naming the bad verb, got: {stderr}"
    );
    let _ = std::fs::remove_dir_all(&parent);
}
