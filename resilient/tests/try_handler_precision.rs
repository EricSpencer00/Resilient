//! Conservative exception paths must not reject a valid implementation.
#![cfg(feature = "z3")]
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
fn run(source: &str, no_typecheck: bool) -> std::process::Output {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let root = std::env::temp_dir().join(format!(
        "machine_try_{}_{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("input.rz"), source).unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_rz"));
    command.args(["--vm", "--audit", "--no-cache"]);
    if no_typecheck {
        command.arg("--no-typecheck");
    }
    let output = command
        .arg(root.join("input.rz"))
        .env_remove("RESILIENT_OVERFLOW_MODE")
        .output()
        .unwrap();
    std::fs::remove_dir_all(root).unwrap();
    output
}
fn assert_one(output: &std::process::Output) {
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter(|line| line.parse::<i64>().is_ok())
            .collect::<Vec<_>>(),
        ["1"],
        "{output:?}"
    );
}
#[test]
fn bounded_unreachable_handler_cannot_hard_refute_valid_try_body() {
    let source = "fn wrong(int x) -> int requires x > 0 ensures result == x { try { return x; } catch Failure { return 0; } } println(wrong(1));";
    let reference = run(source, true);
    assert_one(&reference); // Diagnostic-only execution witness, not accepted proof credit.
    let strict = run(source, false);
    assert!(
        strict.status.success(),
        "strict={strict:?}; diagnostic-only-reference={reference:?}"
    );
    assert_one(&strict);
}
#[test]
fn all_paths_try_contract_remains_useful() {
    assert_one(&run(
        "fn fallback(int x) -> int requires x > 0 ensures result == x || result == 0 { try { return x; } catch Failure { return 0; } } println(fallback(1));",
        false,
    ));
}
#[test]
fn wrong_defined_try_body_still_hard_refutes() {
    let output = run(
        "fn wrong(int x) -> int requires x > 0 ensures result == x { try { return 0; } catch Failure { return x; } }",
        false,
    );
    assert!(!output.status.success(), "{output:?}");
    let diagnostic = String::from_utf8_lossy(&output.stderr);
    assert!(
        diagnostic.contains("ensures") && diagnostic.contains("refut"),
        "{output:?}"
    );
}
#[test]
fn typed_try_fallthrough_remains_rejected() {
    let output = run(
        "fn missing(int x) -> int { try { println(x); } catch Failure { return 1; } }",
        false,
    );
    assert!(!output.status.success(), "{output:?}");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("return"),
        "{output:?}"
    );
}
#[test]
fn typed_try_mixed_handler_return_remains_rejected() {
    let output = run(
        "fn mixed(int x) -> int { try { return x; } catch Failure { return \"bad\"; } }",
        false,
    );
    assert!(!output.status.success(), "{output:?}");
    let diagnostic = String::from_utf8_lossy(&output.stderr);
    assert!(
        diagnostic.contains("return type mismatch") && diagnostic.contains("string"),
        "{output:?}"
    );
}
#[test]
fn typed_try_handler_fallthrough_remains_rejected() {
    let output = run(
        "fn missing(int x) -> int { try { return x; } catch Failure { println(x); } }",
        false,
    );
    assert!(!output.status.success(), "{output:?}");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("return"),
        "{output:?}"
    );
}
#[test]
fn declared_failure_call_has_typed_try_return_flow() {
    assert_one(&run(
        "fn risky(int x) -> int fails Failure { return x; } fn fallback(int x) -> int ensures result == x || result == 0 { try { return risky(x); } catch Failure { return 0; } } println(fallback(1));",
        false,
    ));
}
