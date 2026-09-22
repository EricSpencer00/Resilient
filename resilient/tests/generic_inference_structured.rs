//! RES-4776: generic call-site inference must cover structured AST paths.

use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_rz")
}

fn source_file(source: &str) -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "res_generic_inference_structured_{}_{}.rz",
        std::process::id(),
        id
    ));
    std::fs::write(&path, source).expect("write generic inference source");
    path
}

fn typecheck(source: &str) -> Output {
    let path = source_file(source);
    let output = Command::new(binary())
        .args(["--typecheck-strict"])
        .arg(&path)
        .output()
        .expect("spawn rz --typecheck-strict");
    let _ = std::fs::remove_file(path);
    output
}

#[test]
fn missing_generic_parameter_in_match_arm_is_rejected() {
    let output = typecheck(
        r#"fn project<T, U>(T value) -> T { return value; }
fn choose(int selector) -> int {
    return match selector {
        0 => project(42),
        _ => 0,
    };
}
"#,
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        output.status.code(),
        Some(1),
        "structured call unexpectedly passed:\n{stderr}"
    );
    assert!(
        stderr.contains("cannot infer type for `U` in call to `project`"),
        "missing generic inference diagnostic: {stderr}"
    );
}

#[test]
fn fully_inferred_generic_call_in_match_arm_remains_valid() {
    let output = typecheck(
        r#"fn pair<T, U>(T left, U right) -> T { return left; }
fn choose(int selector) -> int {
    return match selector {
        0 => pair(42, 7),
        _ => 0,
    };
}
"#,
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "valid structured generic call failed:\n{stderr}"
    );
}
