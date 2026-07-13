//! RES-3838: end-to-end smoke tests for package existence verification.
//!
//! Covers the error-path example (hallucinated package rejected) and an
//! end-to-end check that a package declared in a project's
//! `resilient.toml` `[dependencies]` table is accepted. The happy-path
//! std-import example is also exercised by the stdout golden harness;
//! this file adds the stderr-side check the standard harness can't
//! express, and pins the diagnostic text against
//! `examples/package_existence_error.expected.txt` so wording
//! regressions surface in CI.

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rz")
}

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn tmp_dir(tag: &str) -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let p = std::env::temp_dir().join(format!(
        "res_pkg_existence_e2e_{}_{}_{}",
        tag,
        std::process::id(),
        n
    ));
    let _ = fs::remove_dir_all(&p);
    fs::create_dir_all(&p).expect("mkdir tmp");
    p
}

#[test]
fn hallucinated_package_is_rejected_with_pinned_diagnostic() {
    let output = Command::new(bin())
        .arg("examples/package_existence_error.rz")
        .current_dir(manifest_dir())
        .output()
        .expect("failed to spawn resilient");
    assert_ne!(
        output.status.code(),
        Some(0),
        "hallucinated-package example must fail to compile"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);

    let expected_file = manifest_dir().join("examples/package_existence_error.expected.txt");
    let expected = fs::read_to_string(&expected_file)
        .unwrap_or_else(|e| panic!("reading {}: {}", expected_file.display(), e));
    let expected_line = expected.trim_end_matches(&['\n', '\r'][..]);
    assert!(
        stderr.contains(expected_line),
        "stderr did not contain pinned diagnostic line.\n  expected: {expected_line}\n  stderr:\n{stderr}"
    );
}

#[test]
fn declared_manifest_dependency_is_accepted() {
    // A package that's genuinely declared as a project dependency must
    // not be rejected as "hallucinated" even though it isn't a built-in.
    let project = tmp_dir("declared_dep");
    fs::write(
        project.join("resilient.toml"),
        "[package]\nname = \"proj\"\nversion = \"0.1.0\"\n\n[dependencies]\nmylib = { path = \"mylib\" }\n",
    )
    .unwrap();
    // A path dependency must itself have a manifest and a `src/` dir —
    // see `pkg_deps::resolve_path_dep`.
    let dep_dir = project.join("mylib");
    let dep_src = dep_dir.join("src");
    fs::create_dir_all(&dep_src).unwrap();
    fs::write(
        dep_dir.join("resilient.toml"),
        "[package]\nname = \"mylib\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    fs::write(
        dep_src.join("helpers.rz"),
        "pub fn greet() { println(\"hello from mylib\"); }\n",
    )
    .unwrap();

    // Imported declarations are namespaced under the *dependency* name
    // (`mylib`), not the module name (`helpers`) — see
    // `imports::append_imported_stmts`'s `namespace` param.
    let main_rz = project.join("main.rz");
    fs::write(
        &main_rz,
        "use mylib::helpers;\n\nfn main() {\n    mylib::greet();\n}\n\nmain();\n",
    )
    .unwrap();

    let output = Command::new(bin())
        .arg(&main_rz)
        .current_dir(&project)
        .output()
        .expect("failed to spawn resilient");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        output.status.code(),
        Some(0),
        "declared dependency import must not be rejected as unknown; stderr:\n{stderr}"
    );
    assert!(
        !stderr.contains("pkg-existence") && !stderr.contains("unknown package"),
        "declared dependency wrongly flagged as unknown; stderr:\n{stderr}"
    );
}
