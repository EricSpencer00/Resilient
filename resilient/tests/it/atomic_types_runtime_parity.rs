//! RES-3395: atomic_types check rejects runtime-invalid value classes.

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
        "res_atomic_types_runtime_parity_{}_{}_{}",
        tag,
        std::process::id(),
        n
    ));
    std::fs::create_dir_all(&p).expect("mkdir");
    p
}

fn check_source(tag: &str, source: &str) -> (String, String, Option<i32>) {
    let dir = tmp_dir(tag);
    let path = dir.join("case.rz");
    std::fs::write(&path, source).expect("write source");
    let out = Command::new(bin())
        .arg("check")
        .arg(&path)
        .output()
        .expect("spawn rz check");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code(),
    )
}

fn assert_atomic_value_error(tag: &str, source: &str, expected: &str) {
    let (stdout, stderr, code) = check_source(tag, source);
    assert_ne!(
        code,
        Some(0),
        "invalid atomic value should fail; stdout={stdout} stderr={stderr}"
    );
    let combined = format!("{stdout}{stderr}");
    assert!(
        combined.contains(expected),
        "diagnostic must mention `{expected}`; got:\n{combined}"
    );
}

#[test]
fn atomic_store_rejects_runtime_invalid_value_classes() {
    assert_atomic_value_error(
        "float_literal",
        r#"#[atomic]
static let counter = 0;
fn atomic_store(any target, any value) -> int { return 0; }
fn main(int _d) {
    return atomic_store(counter, 1.5);
}
main(0);
"#,
        "atomic_store value must be an integer expression, got float literal",
    );

    assert_atomic_value_error(
        "bool_binding",
        r#"#[atomic]
static let counter = 0;
fn atomic_store(any target, any value) -> int { return 0; }
fn main(int _d) {
    let flag = true;
    return atomic_store(counter, flag);
}
main(0);
"#,
        "atomic_store value must be an integer expression, got boolean binding `flag`",
    );

    assert_atomic_value_error(
        "string_call",
        r#"#[atomic]
static let counter = 0;
fn atomic_store(any target, any value) -> int { return 0; }
fn label() -> string { return "ready"; }
fn main(int _d) {
    return atomic_store(counter, label());
}
main(0);
"#,
        "atomic_store value must be an integer expression, got call returning string",
    );
}

#[test]
fn atomic_fetch_add_rejects_runtime_invalid_value_classes() {
    assert_atomic_value_error(
        "array_binding",
        r#"#[atomic]
static let counter = 0;
fn atomic_fetch_add(any target, any value) -> int { return 0; }
fn main(int _d) {
    let delta = [1];
    return atomic_fetch_add(counter, delta);
}
main(0);
"#,
        "atomic_fetch_add value must be an integer expression, got list binding `delta`",
    );

    assert_atomic_value_error(
        "comparison_expr",
        r#"#[atomic]
static let counter = 0;
fn atomic_fetch_add(any target, any value) -> int { return 0; }
fn main(int _d) {
    return atomic_fetch_add(counter, 1 < 2);
}
main(0);
"#,
        "atomic_fetch_add value must be an integer expression, got boolean expression",
    );
}

#[test]
fn atomic_value_facts_are_scoped_per_function_body() {
    assert_atomic_value_error(
        "shadowed_binding",
        r#"#[atomic]
static let counter = 0;
fn atomic_fetch_add(any target, any value) -> int { return 0; }
fn bad(int _d) {
  let delta = [1];
  return atomic_fetch_add(counter, delta);
}
fn good(int _d) {
  let delta = 1;
  return atomic_fetch_add(counter, delta);
}
bad(0);
"#,
        "atomic_fetch_add value must be an integer expression, got list binding `delta`",
    );
}

#[test]
fn atomic_value_facts_cover_branch_bodies() {
    assert_atomic_value_error(
        "branch_binding",
        r#"#[atomic]
static let counter = 0;
fn atomic_store(any target, any value) -> int { return value; }
fn main(int _d) {
  if true {
    let flag = true;
    return atomic_store(counter, flag);
  }
  return 0;
}
main(0);
"#,
        "atomic_store value must be an integer expression, got boolean binding `flag`",
    );
}
