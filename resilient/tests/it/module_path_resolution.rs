//! RES-4110: dependency module paths support directory `mod.rz` entry points.
//!
//! The package resolver already accepts direct module files and nested
//! directory paths. These end-to-end cases pin the directory-module form at
//! the CLI boundary, where a resolver regression would otherwise surface only
//! as an unrelated import failure.

use std::fs;
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
        "res_module_path_resolution_{}_{}_{}",
        tag,
        std::process::id(),
        n
    ));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).expect("create module-resolution test directory");
    path
}

#[test]
fn single_segment_directory_module_resolves_mod_entry_point() {
    let project = tmp_dir("single");
    let dep = project.join("mylib");
    fs::create_dir_all(&dep).unwrap();
    fs::write(
        project.join("resilient.toml"),
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n[dependencies]\nmylib = { path = \"mylib\" }\n",
    )
    .unwrap();
    fs::write(
        dep.join("resilient.toml"),
        "[package]\nname = \"mylib\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    fs::create_dir_all(dep.join("src/greetings")).unwrap();
    fs::write(
        dep.join("src/greetings/mod.rz"),
        "pub fn greet() { println(\"single directory module\"); }\n",
    )
    .unwrap();
    let main = project.join("main.rz");
    fs::write(
        &main,
        "use mylib::greetings;\n\nfn main() {\n    mylib::greet();\n}\n\nmain();\n",
    )
    .unwrap();

    let output = Command::new(bin())
        .arg(&main)
        .current_dir(&project)
        .output()
        .expect("spawn resilient");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        output.status.code(),
        Some(0),
        "single-segment mod.rz import failed: {stderr}"
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).lines().next(),
        Some("single directory module")
    );

    // Preserve the existing file-based resolution preference when both
    // layouts are present for the same module name.
    fs::write(
        dep.join("src/greetings.rz"),
        "pub fn greet() { println(\"direct module file\"); }\n",
    )
    .unwrap();
    let direct_output = Command::new(bin())
        .arg(&main)
        .current_dir(&project)
        .output()
        .expect("spawn resilient");
    let direct_stderr = String::from_utf8_lossy(&direct_output.stderr);
    assert_eq!(
        direct_output.status.code(),
        Some(0),
        "direct module file import failed: {direct_stderr}"
    );
    assert_eq!(
        String::from_utf8_lossy(&direct_output.stdout)
            .lines()
            .next(),
        Some("direct module file")
    );
    let _ = fs::remove_dir_all(project);
}

#[test]
fn nested_directory_module_resolves_mod_entry_point() {
    let project = tmp_dir("nested");
    let dep = project.join("mylib");
    fs::create_dir_all(&dep).unwrap();
    fs::write(
        project.join("resilient.toml"),
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n[dependencies]\nmylib = { path = \"mylib\" }\n",
    )
    .unwrap();
    fs::write(
        dep.join("resilient.toml"),
        "[package]\nname = \"mylib\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    fs::create_dir_all(dep.join("src/outer/inner")).unwrap();
    fs::write(
        dep.join("src/outer/inner/mod.rz"),
        "pub fn greet() { println(\"nested directory module\"); }\n",
    )
    .unwrap();
    let main = project.join("main.rz");
    fs::write(
        &main,
        "use mylib::outer::inner;\n\nfn main() {\n    mylib::greet();\n}\n\nmain();\n",
    )
    .unwrap();

    let output = Command::new(bin())
        .arg(&main)
        .current_dir(&project)
        .output()
        .expect("spawn resilient");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        output.status.code(),
        Some(0),
        "nested mod.rz import failed: {stderr}"
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).lines().next(),
        Some("nested directory module")
    );
    let _ = fs::remove_dir_all(project);
}
