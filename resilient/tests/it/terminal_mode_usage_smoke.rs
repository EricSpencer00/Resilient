//! RES-3161: terminal-mode usage errors should not print replay seeds.

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rz")
}

fn tmp_file(tag: &str, body: &str) -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "res_3161_terminal_mode_{}_{}_{}.rz",
        tag,
        std::process::id(),
        n
    ));
    std::fs::write(&path, body).expect("write scratch source");
    path
}

#[test]
fn dump_ast_json_missing_path_does_not_print_seed() {
    let output = Command::new(bin())
        .arg("--dump-ast-json")
        .output()
        .expect("spawn rz --dump-ast-json");

    assert_eq!(
        output.status.code(),
        Some(2),
        "missing dump-ast-json path should be a usage error; stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Error: --dump-ast-json requires a path argument")
            && !stderr.contains("seed="),
        "usage error should not be preceded by a replay seed; got:\n{stderr}"
    );
}

#[test]
fn dump_tokens_missing_path_does_not_print_seed() {
    let output = Command::new(bin())
        .arg("--dump-tokens")
        .output()
        .expect("spawn rz --dump-tokens");

    assert_eq!(
        output.status.code(),
        Some(2),
        "missing dump-tokens path should be a usage error; stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Error: --dump-tokens requires a path argument")
            && !stderr.contains("seed="),
        "usage error should not be preceded by a replay seed; got:\n{stderr}"
    );
}

#[test]
fn normal_execution_still_prints_generated_seed() {
    let path = tmp_file("generated_seed", "println(\"res3161 generated seed\");\n");
    let output = Command::new(bin())
        .arg(&path)
        .output()
        .expect("spawn rz program");
    let _ = std::fs::remove_file(&path);

    assert_eq!(
        output.status.code(),
        Some(0),
        "normal execution should still succeed; stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("seed="),
        "normal execution without --seed should still print replay seed; got:\n{stderr}"
    );
}

#[test]
fn explicit_seed_still_suppresses_generated_seed() {
    let path = tmp_file("explicit_seed", "println(\"res3161 explicit seed\");\n");
    let output = Command::new(bin())
        .args(["--seed", "0"])
        .arg(&path)
        .output()
        .expect("spawn rz --seed program");
    let _ = std::fs::remove_file(&path);

    assert_eq!(
        output.status.code(),
        Some(0),
        "execution with explicit seed should still succeed; stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("seed="),
        "--seed should suppress generated seed output; got:\n{stderr}"
    );
}
