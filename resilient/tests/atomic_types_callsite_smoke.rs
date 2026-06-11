//! RES-3393: call-site shape checks for atomic_types operations.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rz")
}

fn tmp_dir(tag: &str) -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let p = std::env::temp_dir().join(format!(
        "res_atomic_types_callsite_{}_{}_{}",
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

fn assert_file_line_col(output: &str, path: &Path, line: usize) {
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

fn assert_atomic_call_error(source: &str, expected: &str, line: usize) {
    let (path, stdout, stderr, code) = check_source("invalid", source);
    assert_eq!(
        code,
        Some(1),
        "invalid atomic call should fail; stdout={stdout} stderr={stderr}"
    );
    let combined = format!("{stdout}{stderr}");
    assert_file_line_col(&combined, &path, line);
    assert!(
        combined.contains(expected),
        "diagnostic must contain `{expected}`; got:\n{combined}"
    );
}

#[test]
fn atomic_call_sites_accept_atomic_identifiers_and_integer_values() {
    let (_path, stdout, stderr, code) = check_source(
        "valid",
        r#"
#[atomic]
static let counter = 0;
fn atomic_load(any target) -> int { return 0; }
fn atomic_store(any target, any value) -> int { return value; }
fn atomic_fetch_add(any target, any value) -> int { return value; }
fn main(int _d) {
    atomic_store(counter, 1);
    let current = atomic_load(counter);
    return current + atomic_fetch_add(counter, 2);
}
main(0);
"#,
    );
    assert_eq!(
        code,
        Some(0),
        "valid atomic call sites should pass; stdout={stdout} stderr={stderr}"
    );
}

#[test]
fn atomic_load_rejects_string_number_list_and_struct_targets() {
    let prefix = r#"
#[atomic]
static let counter = 0;
struct Cell { int value }
fn atomic_load(any target) -> int { return 0; }
"#;

    assert_atomic_call_error(
        &format!("{prefix}fn main(int _d) {{ return atomic_load(\"counter\"); }}\nmain(0);\n"),
        "atomic_load target must be an atomic identifier, got string literal",
        6,
    );
    assert_atomic_call_error(
        &format!("{prefix}fn main(int _d) {{ return atomic_load(1); }}\nmain(0);\n"),
        "atomic_load target must be an atomic identifier, got number literal",
        6,
    );
    assert_atomic_call_error(
        &format!("{prefix}fn main(int _d) {{ return atomic_load([counter]); }}\nmain(0);\n"),
        "atomic_load target must be an atomic identifier, got list literal",
        6,
    );
    assert_atomic_call_error(
        &format!(
            "{prefix}fn main(int _d) {{ return atomic_load(new Cell {{ value: 0 }}); }}\nmain(0);\n"
        ),
        "atomic_load target must be an atomic identifier, got struct literal",
        6,
    );
}

#[test]
fn atomic_store_and_fetch_add_validate_arity_and_integer_values() {
    assert_atomic_call_error(
        r#"
#[atomic]
static let counter = 0;
fn atomic_store(any target) -> int { return 0; }
fn main(int _d) { return atomic_store(counter); }
main(0);
"#,
        "atomic_store expects 2 arguments, got 1",
        5,
    );
    assert_atomic_call_error(
        r#"
#[atomic]
static let counter = 0;
fn atomic_fetch_add(any target, any value) -> int { return value; }
fn main(int _d) { return atomic_fetch_add(counter, [1]); }
main(0);
"#,
        "atomic_fetch_add value must be an integer expression, got list literal",
        5,
    );
}
