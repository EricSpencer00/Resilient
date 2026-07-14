//! RES-4032 (E-E5): `rz fmt --check` — CI/pre-commit mode.
//!
//! Verifies the exit-code contract (`cargo fmt --check` / `rustfmt
//! --check` UX): 0 when every file is already canonically formatted,
//! non-zero when any file would change, and the file is never written
//! regardless of outcome.

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rz")
}

fn tmp_dir(tag: &str) -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let p = std::env::temp_dir().join(format!(
        "res_fmt_check_smoke_{}_{}_{}",
        tag,
        std::process::id(),
        n
    ));
    std::fs::create_dir_all(&p).expect("mkdir");
    p
}

const FORMATTED: &str = "\
fn main() {
    println(\"hi\");
}

main();
";

const MISFORMATTED: &str = "fn  main(  )   {\n  println(\"hi\") ;\n}\n\nmain( ) ;\n";

#[test]
fn check_exits_zero_on_already_formatted_file() {
    let dir = tmp_dir("clean");
    let path = dir.join("clean.rz");
    std::fs::write(&path, FORMATTED).expect("write source");

    let out = Command::new(bin())
        .args(["fmt", "--check"])
        .arg(&path)
        .output()
        .expect("spawn rz fmt --check");

    assert_eq!(
        out.status.code(),
        Some(0),
        "already-formatted file should exit 0; stdout={} stderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        out.stdout.is_empty(),
        "--check must print nothing to stdout on success: {:?}",
        String::from_utf8_lossy(&out.stdout)
    );

    let after = std::fs::read_to_string(&path).expect("reread file");
    assert_eq!(after, FORMATTED, "--check must never write the file");
}

#[test]
fn check_exits_nonzero_on_misformatted_file_and_does_not_write() {
    let dir = tmp_dir("dirty");
    let path = dir.join("dirty.rz");
    std::fs::write(&path, MISFORMATTED).expect("write source");

    let out = Command::new(bin())
        .args(["fmt", "--check"])
        .arg(&path)
        .output()
        .expect("spawn rz fmt --check");

    assert_ne!(
        out.status.code(),
        Some(0),
        "misformatted file should exit non-zero; stdout={} stderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        out.stdout.is_empty(),
        "--check must print nothing to stdout even on failure: {:?}",
        String::from_utf8_lossy(&out.stdout)
    );

    let after = std::fs::read_to_string(&path).expect("reread file");
    assert_eq!(
        after, MISFORMATTED,
        "--check must never write the file, even when it would reformat"
    );
}

#[test]
fn check_reports_multiple_files_and_fails_if_any_is_dirty() {
    let dir = tmp_dir("multi");
    let clean_path = dir.join("clean.rz");
    let dirty_path = dir.join("dirty.rz");
    std::fs::write(&clean_path, FORMATTED).expect("write clean source");
    std::fs::write(&dirty_path, MISFORMATTED).expect("write dirty source");

    let out = Command::new(bin())
        .args(["fmt", "--check"])
        .arg(&clean_path)
        .arg(&dirty_path)
        .output()
        .expect("spawn rz fmt --check");

    assert_ne!(
        out.status.code(),
        Some(0),
        "a set with any dirty file should exit non-zero"
    );
    assert!(
        out.stdout.is_empty(),
        "--check must print nothing to stdout"
    );
    assert_eq!(
        std::fs::read_to_string(&clean_path).expect("reread clean"),
        FORMATTED
    );
    assert_eq!(
        std::fs::read_to_string(&dirty_path).expect("reread dirty"),
        MISFORMATTED
    );
}

#[test]
fn check_all_clean_set_exits_zero() {
    let dir = tmp_dir("multi_clean");
    let a = dir.join("a.rz");
    let b = dir.join("b.rz");
    std::fs::write(&a, FORMATTED).expect("write a");
    std::fs::write(&b, FORMATTED).expect("write b");

    let out = Command::new(bin())
        .args(["fmt", "--check"])
        .arg(&a)
        .arg(&b)
        .output()
        .expect("spawn rz fmt --check");

    assert_eq!(out.status.code(), Some(0));
    assert!(out.stdout.is_empty());
}

#[test]
fn check_requires_at_least_one_file() {
    let out = Command::new(bin())
        .args(["fmt", "--check"])
        .output()
        .expect("spawn rz fmt --check");

    assert_eq!(
        out.status.code(),
        Some(2),
        "missing file path is a usage error"
    );
}

#[test]
fn check_rejects_combination_with_in_place() {
    let dir = tmp_dir("conflict");
    let path = dir.join("case.rz");
    std::fs::write(&path, FORMATTED).expect("write source");

    let out = Command::new(bin())
        .args(["fmt", "--check", "--in-place"])
        .arg(&path)
        .output()
        .expect("spawn rz fmt --check --in-place");

    assert_eq!(
        out.status.code(),
        Some(2),
        "--check + --in-place is a usage error, not silently one-or-the-other"
    );
    assert_eq!(
        std::fs::read_to_string(&path).expect("reread file"),
        FORMATTED,
        "the conflicting invocation must not write the file"
    );
}
