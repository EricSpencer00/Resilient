//! RES-4007: `rz pkg remove <name>` end-to-end smoke tests.

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
        "res_pkg_remove_{}_{}_{}",
        tag,
        std::process::id(),
        n
    ));
    std::fs::create_dir_all(&p).expect("mkdir pkg remove tmp");
    p
}

fn scaffold_app_with_dep(parent: &std::path::Path, dep_name: &str) -> PathBuf {
    let app = parent.join("app");
    let dep = parent.join("libs").join(dep_name);
    std::fs::create_dir_all(app.join("src")).expect("mkdir app src");
    std::fs::create_dir_all(dep.join("src")).expect("mkdir dep src");
    std::fs::write(
        app.join("resilient.toml"),
        format!(
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n[dependencies]\n{} = {{ path = \"../libs/{}\" }}\n",
            dep_name, dep_name
        ),
    )
    .expect("write app manifest");
    std::fs::write(
        dep.join("resilient.toml"),
        format!("[package]\nname = \"{}\"\nversion = \"0.1.0\"\n", dep_name),
    )
    .expect("write dep manifest");
    app
}

#[test]
fn pkg_remove_drops_dependency_and_rewrites_lockfile() {
    let parent = tmp_parent("basic");
    let app = scaffold_app_with_dep(&parent, "mylib");

    // Seed a lockfile the way `pkg add` would have.
    let add_output = Command::new(bin())
        .args(["pkg", "remove", "mylib"])
        .current_dir(&app)
        .output()
        .expect("spawn rz pkg remove mylib");

    assert!(
        add_output.status.success(),
        "expected pkg remove to succeed; stdout={} stderr={}",
        String::from_utf8_lossy(&add_output.stdout),
        String::from_utf8_lossy(&add_output.stderr)
    );

    let manifest = std::fs::read_to_string(app.join("resilient.toml")).expect("read manifest");
    assert!(
        !manifest.contains("mylib"),
        "mylib should be removed from manifest: {}",
        manifest
    );

    let lock = std::fs::read_to_string(app.join("resilient.lock")).expect("read lockfile");
    assert!(
        !lock.contains("mylib"),
        "mylib should be removed from lockfile: {}",
        lock
    );

    let _ = std::fs::remove_dir_all(&parent);
}

#[test]
fn pkg_remove_errors_cleanly_when_dependency_absent() {
    let parent = tmp_parent("absent");
    let app = parent.join("app");
    std::fs::create_dir_all(app.join("src")).expect("mkdir app src");
    std::fs::write(
        app.join("resilient.toml"),
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n[dependencies]\n",
    )
    .expect("write app manifest");

    let output = Command::new(bin())
        .args(["pkg", "remove", "nope"])
        .current_dir(&app)
        .output()
        .expect("spawn rz pkg remove nope");

    assert_eq!(
        output.status.code(),
        Some(1),
        "removing an absent dependency should fail cleanly; stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("no dependency named `nope`"),
        "expected a clear not-found message; got:\n{stderr}"
    );

    let _ = std::fs::remove_dir_all(&parent);
}

#[test]
fn pkg_remove_requires_a_name() {
    let output = Command::new(bin())
        .args(["pkg", "remove"])
        .output()
        .expect("spawn rz pkg remove (no args)");

    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("requires a dependency name"), "{stderr}");
}

#[test]
fn pkg_remove_help_is_focused() {
    let output = Command::new(bin())
        .args(["pkg", "remove", "--help"])
        .output()
        .expect("spawn rz pkg remove --help");

    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("rz pkg remove"));
    assert!(stdout.contains("USAGE:\n    rz pkg remove <name>"));
}
