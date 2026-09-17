use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rz")
}

fn tmp_dir(tag: &str) -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "res_package_manager_validation_{}_{}_{}",
        tag,
        std::process::id(),
        n
    ));
    fs::create_dir_all(&path).expect("create temporary project directory");
    path
}

fn write_source(project: &Path) -> PathBuf {
    let source = project.join("main.rz");
    fs::write(
        &source,
        "fn main() { println(\"package check\"); }\nmain();\n",
    )
    .expect("write source");
    source
}

#[test]
fn manifest_runtime_failure_is_rejected_during_typecheck() {
    let project = tmp_dir("self_dependency");
    fs::write(
        project.join("rz.toml"),
        "[package]\nname = \"mypkg\"\nversion = \"1.0.0\"\n\n[dependencies]\nmypkg = \"^1.0.0\"\n",
    )
    .expect("write manifest");
    let source = write_source(&project);

    let output = Command::new(bin())
        .arg("check")
        .arg(&source)
        .output()
        .expect("run rz check");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        output.status.code(),
        Some(1),
        "self-dependent package must fail during typecheck; stderr={stderr}"
    );
    assert!(
        stderr.contains("error[pkg]: package cannot depend on itself: `mypkg`"),
        "expected package diagnostic; stderr={stderr}"
    );

    let _ = fs::remove_dir_all(project);
}

#[test]
fn valid_manifest_still_allows_typecheck() {
    let project = tmp_dir("valid");
    fs::write(
        project.join("resilient.toml"),
        "[package]\nname = \"app\"\nversion = \"1.0.0\"\n\n[dependencies]\ntelemetry = \"^2.1.0\"\n",
    )
    .expect("write manifest");
    let source = write_source(&project);

    let output = Command::new(bin())
        .arg("check")
        .arg(&source)
        .output()
        .expect("run rz check");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        output.status.code(),
        Some(0),
        "valid manifest must pass typecheck; stderr={stderr}"
    );
    assert!(
        stderr.contains("pkg: manifest `app` v1.0.0 with 1 dependency/ies validated"),
        "valid manifest should emit its validation notice; stderr={stderr}"
    );

    let _ = fs::remove_dir_all(project);
}
