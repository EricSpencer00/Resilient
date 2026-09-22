//! RES-4593: ghost-call checks must cover every executable AST shape.

use std::path::PathBuf;
use std::process::Output;
use std::sync::atomic::{AtomicUsize, Ordering};

fn scratch_file(source: &str) -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "res_ghost_call_boundaries_{}_{}.rz",
        std::process::id(),
        id
    ));
    std::fs::write(&path, source).expect("write ghost-call regression source");
    path
}

fn run_typecheck(source: &str) -> Output {
    let path = scratch_file(source);
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_rz"))
        .args(["--typecheck-strict"])
        .arg(&path)
        .output()
        .expect("spawn rz --typecheck-strict");
    let _ = std::fs::remove_file(path);
    output
}

fn assert_ghost_call_rejected(source: &str) {
    let output = run_typecheck(source);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_ne!(
        output.status.code(),
        Some(0),
        "runtime ghost call unexpectedly passed:\nstdout={:?}\nstderr={stderr}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(
        stderr.contains("calls ghost fn `spec_helper`"),
        "missing ghost-call diagnostic:\n{stderr}"
    );
}

#[test]
fn ghost_call_in_while_body_is_rejected() {
    assert_ghost_call_rejected(
        r#"
#[ghost]
fn spec_helper(int value) -> bool { return value > 0; }

fn runtime(int value) -> bool {
    while value > 0 {
        let checked = spec_helper(value);
        return checked;
    }
    return false;
}

runtime(1);
"#,
    );
}

#[test]
fn ghost_call_inside_expression_is_rejected() {
    assert_ghost_call_rejected(
        r#"
#[ghost]
fn spec_helper(int value) -> bool { return value > 0; }

fn runtime(int value) -> bool {
    return value > 0 && spec_helper(value);
}

runtime(1);
"#,
    );
}

#[test]
fn ghost_call_inside_function_literal_is_rejected() {
    assert_ghost_call_rejected(
        r#"
#[ghost]
fn spec_helper(int value) -> bool { return value > 0; }

fn runtime(int value) -> bool {
    let callback = fn(int input) -> bool { return spec_helper(input); };
    return callback(value);
}

runtime(1);
"#,
    );
}

#[test]
fn ghost_functions_may_call_other_ghost_functions() {
    let output = run_typecheck(
        r#"
#[ghost]
fn spec_helper(int value) -> bool { return value > 0; }

#[ghost]
fn spec_wrapper(int value) -> bool { return spec_helper(value); }

spec_wrapper(1);
"#,
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        output.status.code(),
        Some(0),
        "ghost-to-ghost call should typecheck:\n{stderr}"
    );
}
