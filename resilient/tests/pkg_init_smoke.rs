//! RES-205 / RES-212: integration test for the `resilient pkg init`
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
    assert!(root.join("resilient.toml").is_file(), "manifest missing");
    assert!(root.join("src/main.res").is_file(), "entry point missing");
    assert!(root.join(".gitignore").is_file(), "gitignore missing");

    // Manifest content — pin [package] + name + [dependencies] to
    // catch template drift.
    let manifest = std::fs::read_to_string(root.join("resilient.toml")).expect("read manifest");
    assert!(
        manifest.contains("[package]"),
        "missing [package]: {manifest}"
    );
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
    assert!(
        manifest.contains("author = "),
        "missing author field in: {manifest}"
    );
    assert!(
        manifest.contains("[dependencies]"),
        "missing [dependencies] table in: {manifest}"
    );

    // Entry point has the hello-world greeting.
    let main_src = std::fs::read_to_string(root.join("src/main.res")).expect("read main.res");
    assert!(
        main_src.contains("Hello, world!"),
        "missing greeting in: {main_src}"
    );

    // Gitignore ignores target/ and cert/.
    let gi = std::fs::read_to_string(root.join(".gitignore")).expect("read gitignore");
    assert!(gi.contains("target/"), "missing target/ in: {gi}");
    assert!(gi.contains("cert/"), "missing cert/ in: {gi}");

    let _ = std::fs::remove_dir_all(&parent);
}

#[test]
fn pkg_init_accepts_name_flag() {
    // RES-212: the `--name foo` flag is the non-interactive path
    // for automation. Equivalent to the positional `pkg init foo`.
    let parent = tmp_parent("nameflag");
    let output = Command::new(bin())
        .args(["pkg", "init", "--name", "via_flag"])
        .current_dir(&parent)
        .output()
        .expect("spawn resilient pkg init --name");

    assert!(
        output.status.success(),
        "expected exit 0, got {:?}\nstdout: {}\nstderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    let root = parent.join("via_flag");
    let manifest = std::fs::read_to_string(root.join("resilient.toml")).expect("read manifest");
    assert!(
        manifest.contains(r#"name = "via_flag""#),
        "missing name in: {manifest}"
    );
    let _ = std::fs::remove_dir_all(&parent);
}

#[test]
fn pkg_init_name_flag_equals_form() {
    // `--name=foo` should work identically to `--name foo`.
    let parent = tmp_parent("nameeq");
    let output = Command::new(bin())
        .args(["pkg", "init", "--name=via_equals"])
        .current_dir(&parent)
        .output()
        .expect("spawn resilient pkg init --name=");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(parent.join("via_equals/resilient.toml").exists());
    let _ = std::fs::remove_dir_all(&parent);
}

#[test]
fn pkg_init_errors_on_existing_manifest() {
    // RES-212 idempotency guard: if `resilient.toml` already lives
    // in the target, we bail with `ManifestExists`, not the general
    // non-empty-dir error. The stderr should hint at the remove-
    // then-reinit recovery path.
    let parent = tmp_parent("existing_manifest");
    let target = parent.join("seeded");
    std::fs::create_dir(&target).unwrap();
    std::fs::write(
        target.join("resilient.toml"),
        "[package]\nname = \"seeded\"\n",
    )
    .unwrap();

    let output = Command::new(bin())
        .args(["pkg", "init", "seeded"])
        .current_dir(&parent)
        .output()
        .expect("spawn resilient pkg init");

    assert!(
        !output.status.success(),
        "expected non-zero exit. stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("existing manifest") || stderr.contains("overwrite"),
        "expected existing-manifest diagnostic, got stderr: {stderr}"
    );
    // Pre-existing manifest content untouched.
    assert_eq!(
        std::fs::read_to_string(target.join("resilient.toml")).unwrap(),
        "[package]\nname = \"seeded\"\n",
    );
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
        !target.join("resilient.toml").exists(),
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
        stderr.contains("requires a project name") || stderr.contains("<name>"),
        "expected usage hint, got: {stderr}"
    );
    let _ = std::fs::remove_dir_all(&parent);
}

#[test]
fn pkg_help_lists_subcommands() {
    // RES-212: `resilient pkg --help` should print a catalog of
    // subcommands and exit 0.
    let parent = tmp_parent("help");
    let output = Command::new(bin())
        .args(["pkg", "--help"])
        .current_dir(&parent)
        .output()
        .expect("spawn resilient pkg --help");

    assert!(
        output.status.success(),
        "expected exit 0, got {:?}\nstderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr),
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("init"),
        "help output missing `init` subcommand: {stdout}"
    );
    assert!(
        stdout.to_lowercase().contains("subcommand"),
        "help output missing subcommand marker: {stdout}"
    );
    let _ = std::fs::remove_dir_all(&parent);
}

#[test]
fn pkg_bare_prints_help_on_stderr() {
    // `resilient pkg` with no subcommand is a usage error (exit 2)
    // but we still print the subcommand list to stderr so users can
    // self-correct without a second invocation.
    let parent = tmp_parent("bare_pkg");
    let output = Command::new(bin())
        .arg("pkg")
        .current_dir(&parent)
        .output()
        .expect("spawn resilient pkg");
    assert!(!output.status.success(), "expected non-zero exit");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("init"), "expected init in stderr: {stderr}");
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

#[test]
fn run_prefixes_errors_with_package_name() {
    // RES-212: when a `resilient.toml` manifest sits beside the
    // source file (or up a parent chain), a failing `resilient run`
    // should prefix the stderr diagnostic with `[<package-name>] `.
    //
    // We deliberately write a source file that references an
    // undefined identifier so the interpreter / compiler fails
    // fast — no need to exercise a full error path.
    let parent = tmp_parent("run_pkg_prefix");
    let proj = parent.join("broken_proj");
    std::fs::create_dir_all(proj.join("src")).unwrap();
    std::fs::write(
        proj.join("resilient.toml"),
        "[package]\nname = \"my_pkg_212\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    // Reference an undefined identifier so runtime / compile bails.
    let src_path = proj.join("src/main.res");
    std::fs::write(
        &src_path,
        "fn main(int _d) {\n    println(not_a_real_thing);\n    return 0;\n}\nmain(0);\n",
    )
    .unwrap();

    let output = Command::new(bin())
        .arg(&src_path)
        .current_dir(&proj)
        .output()
        .expect("spawn resilient run");

    assert!(
        !output.status.success(),
        "expected non-zero exit from broken program. stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("[my_pkg_212]"),
        "expected [my_pkg_212] package-name prefix in stderr: {stderr}"
    );

    let _ = std::fs::remove_dir_all(&parent);
}
