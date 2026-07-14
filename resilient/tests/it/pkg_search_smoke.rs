//! RES-4007: `rz pkg search <query>` end-to-end smoke tests.

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
        "res_pkg_search_{}_{}_{}",
        tag,
        std::process::id(),
        n
    ));
    std::fs::create_dir_all(&p).expect("mkdir pkg search tmp");
    p
}

#[test]
fn pkg_search_finds_manifest_matches() {
    let parent = tmp_parent("finds_match");
    let app = parent.join("app");
    std::fs::create_dir_all(app.join("src")).expect("mkdir app src");
    std::fs::write(
        app.join("resilient.toml"),
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n[dependencies]\n\
         mylib = { path = \"../libs/mylib\" }\n\
         netutil = { git = \"https://ex.com/netutil\", rev = \"abc123\" }\n",
    )
    .expect("write app manifest");

    let output = Command::new(bin())
        .args(["pkg", "search", "lib"])
        .current_dir(&app)
        .output()
        .expect("spawn rz pkg search lib");

    assert_eq!(
        output.status.code(),
        Some(0),
        "pkg search should succeed; stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("mylib"), "got: {stdout}");
    assert!(!stdout.contains("netutil"), "got: {stdout}");
    assert!(
        stdout.contains("no remote registry index exists yet"),
        "should note registry search is future work; got:\n{stdout}"
    );

    let _ = std::fs::remove_dir_all(&parent);
}

#[test]
fn pkg_search_reports_no_local_matches() {
    let parent = tmp_parent("no_match");
    let app = parent.join("app");
    std::fs::create_dir_all(app.join("src")).expect("mkdir app src");
    std::fs::write(
        app.join("resilient.toml"),
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n[dependencies]\n",
    )
    .expect("write app manifest");

    let output = Command::new(bin())
        .args(["pkg", "search", "nonexistent"])
        .current_dir(&app)
        .output()
        .expect("spawn rz pkg search nonexistent");

    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("No local matches"), "got: {stdout}");
    assert!(stdout.contains("no remote registry index exists yet"));

    let _ = std::fs::remove_dir_all(&parent);
}

#[test]
fn pkg_search_requires_a_query() {
    let output = Command::new(bin())
        .args(["pkg", "search"])
        .output()
        .expect("spawn rz pkg search (no args)");

    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("requires a query"), "{stderr}");
}

#[test]
fn pkg_search_help_is_focused() {
    let output = Command::new(bin())
        .args(["pkg", "search", "--help"])
        .output()
        .expect("spawn rz pkg search --help");

    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("rz pkg search"));
    assert!(stdout.contains("USAGE:\n    rz pkg search <query>"));
}
