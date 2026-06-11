//! RES-3392: end-to-end checks for `#[atomic]` declaration validation.

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
        "res_atomic_types_{}_{}_{}",
        tag,
        std::process::id(),
        n
    ));
    std::fs::create_dir_all(&p).expect("mkdir");
    p
}

fn check_source(tag: &str, source: &str) -> (PathBuf, String, String, Option<i32>) {
    let dir = tmp_dir(tag);
    let path = dir.join("case.rz");
    std::fs::write(&path, source).expect("write source");
    let out = Command::new(bin())
        .arg("check")
        .arg(&path)
        .output()
        .expect("spawn rz check");
    (
        path,
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code(),
    )
}

#[test]
fn atomic_static_let_typechecks() {
    let (_path, stdout, stderr, code) = check_source(
        "valid",
        "#[atomic]\nstatic let counter = 0;\nfn main(int _d) { return 0; }\nmain(0);\n",
    );
    assert_eq!(
        code,
        Some(0),
        "valid atomic static let should pass; stdout={stdout} stderr={stderr}"
    );
}

#[test]
fn atomic_on_function_is_rejected() {
    let (path, stdout, stderr, code) = check_source(
        "function_shape",
        "#[atomic]\nfn counter(int x) -> int { return x; }\ncounter(0);\n",
    );
    assert_eq!(
        code,
        Some(1),
        "atomic function shape must fail; stdout={stdout} stderr={stderr}"
    );
    let combined = format!("{stdout}{stderr}");
    assert_file_line_col(&combined, &path, 2);
    assert!(
        combined.contains("`counter`") && combined.contains("static let"),
        "diagnostic must name the malformed atomic declaration; got:\n{combined}"
    );
}

#[test]
fn atomic_attribute_arguments_are_rejected() {
    let (path, stdout, stderr, code) = check_source(
        "args",
        "#[atomic(width = 64)]\nstatic let counter = 0;\nfn main(int _d) { return 0; }\nmain(0);\n",
    );
    assert_eq!(
        code,
        Some(1),
        "atomic attributes with args must fail; stdout={stdout} stderr={stderr}"
    );
    let combined = format!("{stdout}{stderr}");
    assert_file_line_col(&combined, &path, 2);
    assert!(
        combined.contains("#[atomic]") && combined.contains("does not accept arguments"),
        "diagnostic must reject malformed atomic attributes; got:\n{combined}"
    );
}

#[test]
fn atomic_non_integer_initializer_is_rejected() {
    let (path, stdout, stderr, code) = check_source(
        "non_integer",
        "#[atomic]\nstatic let flag = true;\nfn main(int _d) { return 0; }\nmain(0);\n",
    );
    assert_eq!(
        code,
        Some(1),
        "atomic non-integer initializer must fail; stdout={stdout} stderr={stderr}"
    );
    let combined = format!("{stdout}{stderr}");
    assert_file_line_col(&combined, &path, 2);
    assert!(
        combined.contains("`flag`") && combined.contains("integer literal"),
        "diagnostic must reject non-integer atomic initializer; got:\n{combined}"
    );
}

fn assert_file_line_col(output: &str, path: &std::path::Path, line: usize) {
    let prefix = format!("{}:{}:", path.display(), line);
    let has_line_col = output.lines().any(|line| {
        line.strip_prefix(&prefix)
            .and_then(|rest| rest.split_once(':'))
            .is_some_and(|(col, _)| col.parse::<usize>().is_ok())
    });
    assert!(
        has_line_col,
        "diagnostic must include file:line:col; got:\n{output}"
    );
}
