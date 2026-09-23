//! RES-4599: zero-major caret dependency ranges use semver compatibility bounds.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rz")
}

fn temp_project(tag: &str) -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "res_package_manager_caret_{}_{}_{}",
        tag,
        std::process::id(),
        n
    ));
    fs::create_dir_all(&path).expect("create temporary project directory");
    path
}

fn run_check(project: &Path, constraint: &str, locked_version: &str) -> std::process::Output {
    fs::write(
        project.join("rz.toml"),
        format!(
            "[package]\nname = \"app\"\nversion = \"1.0.0\"\n\n[dependencies]\nlib = \"{constraint}\"\n"
        ),
    )
    .expect("write manifest");
    fs::write(
        project.join("resilient.lock"),
        format!("lib = \"{locked_version}\"\n"),
    )
    .expect("write lockfile");
    let source = project.join("main.rz");
    fs::write(&source, "fn main() { println(\"ok\"); }\nmain();\n").expect("write source");

    Command::new(bin())
        .arg("check")
        .arg(source)
        .output()
        .expect("run rz check")
}

#[test]
fn zero_major_caret_ranges_stop_at_first_nonzero_component() {
    let project = temp_project("minor");
    let accepted = run_check(&project, "^0.2.3", "0.2.9");
    assert!(
        accepted.status.success(),
        "^0.2.3 should accept 0.2.9: {}",
        String::from_utf8_lossy(&accepted.stderr)
    );

    let rejected = run_check(&project, "^0.2.3", "0.3.0");
    assert_eq!(rejected.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&rejected.stderr).contains("does not satisfy constraint `^0.2.3`"),
        "expected zero-major minor bound diagnostic: {}",
        String::from_utf8_lossy(&rejected.stderr)
    );
    let _ = fs::remove_dir_all(project);
}

#[test]
fn zero_zero_caret_ranges_stop_at_next_patch() {
    let project = temp_project("patch");
    let accepted = run_check(&project, "^0.0.3", "0.0.3");
    assert!(
        accepted.status.success(),
        "^0.0.3 should accept itself: {}",
        String::from_utf8_lossy(&accepted.stderr)
    );

    let rejected = run_check(&project, "^0.0.0", "0.0.0");
    assert!(
        rejected.status.success(),
        "^0.0.0 should accept itself: {}",
        String::from_utf8_lossy(&rejected.stderr)
    );
    let rejected = run_check(&project, "^0.0.3", "0.0.4");
    assert_eq!(rejected.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&rejected.stderr).contains("does not satisfy constraint `^0.0.3`"),
        "expected zero-zero patch bound diagnostic: {}",
        String::from_utf8_lossy(&rejected.stderr)
    );
    let _ = fs::remove_dir_all(project);
}

#[test]
fn non_zero_caret_ranges_keep_major_compatibility() {
    let project = temp_project("major");
    let accepted = run_check(&project, "^1.2.3", "1.5.0");
    assert!(
        accepted.status.success(),
        "^1.2.3 should accept 1.5.0: {}",
        String::from_utf8_lossy(&accepted.stderr)
    );

    let rejected = run_check(&project, "^1.2.3", "2.0.0");
    assert_eq!(rejected.status.code(), Some(1));
    let _ = fs::remove_dir_all(project);
}
