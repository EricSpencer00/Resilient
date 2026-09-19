use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rz")
}

fn scratch_file(tag: &str, source: &str) -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let path =
        std::env::temp_dir().join(format!("res_4110_{tag}_{}_{}.rz", std::process::id(), id));
    std::fs::write(&path, source).expect("write inline glob source");
    path
}

#[test]
fn imports_public_inline_module_items() {
    let path = scratch_file(
        "public",
        r#"
mod math {
    pub fn add(int left, int right) -> int { return left + right; }
    fn hidden() -> int { return 99; }
    pub struct Point { int x, int y }
}
use math::*;
fn main() { println(add(2, 3)); }
main();
"#,
    );
    let output = Command::new(bin())
        .arg(&path)
        .output()
        .expect("run glob import");
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        output.status.code(),
        Some(0),
        "glob import failed: {stderr}"
    );
    assert!(
        stdout.contains("5"),
        "expected imported add() result, got: {stdout}"
    );
    assert!(
        !stderr.contains("Parser error"),
        "unexpected parser error: {stderr}"
    );
    let _ = std::fs::remove_file(path);
}

#[test]
fn private_inline_module_items_are_not_imported() {
    let path = scratch_file(
        "private",
        r#"
mod secret {
    fn hidden() -> int { return 7; }
}
use secret::*;
fn main() { println(hidden()); }
main();
"#,
    );
    let output = Command::new(bin())
        .arg("--typecheck")
        .arg(&path)
        .output()
        .expect("typecheck private glob import");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_ne!(
        output.status.code(),
        Some(0),
        "private item was imported: {stderr}"
    );
    assert!(
        stderr.contains("hidden"),
        "diagnostic should name hidden: {stderr}"
    );
    let _ = std::fs::remove_file(path);
}

#[test]
fn duplicate_inline_glob_exports_are_rejected() {
    let path = scratch_file(
        "ambiguous",
        r#"
mod left { pub fn same() -> int { return 1; } }
mod right { pub fn same() -> int { return 2; } }
use left::*;
use right::*;
fn main() { println(same()); }
main();
"#,
    );
    let output = Command::new(bin())
        .arg(&path)
        .output()
        .expect("run ambiguous glob import");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_ne!(
        output.status.code(),
        Some(0),
        "ambiguous glob import succeeded"
    );
    assert!(
        stderr.contains("ambiguous"),
        "expected ambiguity diagnostic: {stderr}"
    );
    assert!(
        stderr.contains("same"),
        "diagnostic should name the collision: {stderr}"
    );
    let _ = std::fs::remove_file(path);
}
