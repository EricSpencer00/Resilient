use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rz")
}

fn tmp_dir(tag: &str) -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "res3213_const_eval_ext_rejections_{}_{}_{}",
        tag,
        std::process::id(),
        n
    ));
    std::fs::create_dir_all(&dir).expect("mkdir");
    dir
}

fn check_err(src: &str) -> String {
    let dir = tmp_dir("check");
    let path = dir.join("bad.rz");
    std::fs::write(&path, src).expect("write source");

    let output = Command::new(bin())
        .arg("check")
        .arg(&path)
        .output()
        .expect("spawn rz check");

    assert!(
        !output.status.success(),
        "expected check failure, stdout={}, stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[test]
fn const_eval_ext_rejects_function_calls() {
    let err = check_err(
        r#"
fn id(int x) {
    return x;
}

const VALUE = id(1);
"#,
    );
    assert!(
        err.contains("invalid const expression: function calls are not allowed"),
        "unexpected error: {err}"
    );
}

#[test]
fn const_eval_ext_rejects_array_literals() {
    let err = check_err(
        r#"
const VALUE = [1, 2];
"#,
    );
    assert!(
        err.contains("invalid const expression: array literals are not allowed"),
        "unexpected error: {err}"
    );
}

#[test]
fn const_eval_ext_rejects_struct_literals() {
    let err = check_err(
        r#"
struct Point { int x, int y }

const VALUE = new Point { x: 1, y: 2 };
"#,
    );
    assert!(
        err.contains("invalid const expression: struct literals are not allowed"),
        "unexpected error: {err}"
    );
}

#[test]
fn const_eval_ext_rejects_match_expressions() {
    let err = check_err(
        r#"
const VALUE = match 1 {
    1 => 2,
    _ => 3,
};
"#,
    );
    assert!(
        err.contains("invalid const expression: match expressions are not allowed"),
        "unexpected error: {err}"
    );
}

#[test]
fn const_eval_ext_rejects_multi_statement_blocks() {
    let err = check_err(
        r#"
const VALUE = if true {
    1;
    2;
} else {
    3;
};
"#,
    );
    assert!(
        err.contains(
            "invalid const expression: blocks in const initializers must contain exactly one expression"
        ),
        "unexpected error: {err}"
    );
}
