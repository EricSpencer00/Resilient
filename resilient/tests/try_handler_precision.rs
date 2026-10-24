//! Typed try return flow must preserve language guards and avoid proof claims for calls.
#![cfg(feature = "z3")]
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
fn run_captured(
    source: &str,
    no_typecheck: bool,
    report: bool,
) -> (std::process::Output, Option<serde_json::Value>) {
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
    if report {
        command
            .arg("--emit-contract-certificate")
            .arg(root.join("report.json"));
    }
    let output = command
        .arg(root.join("input.rz"))
        .env_remove("RESILIENT_OVERFLOW_MODE")
        .output()
        .unwrap();
    let report = std::fs::read(root.join("report.json"))
        .ok()
        .map(|bytes| serde_json::from_slice(&bytes).unwrap());
    std::fs::remove_dir_all(root).unwrap();
    (output, report)
}
fn run(source: &str, no_typecheck: bool) -> std::process::Output {
    run_captured(source, no_typecheck, false).0
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
fn pure_dead_catch_source_is_not_accepted() {
    let source = "fn wrong(int x) -> int requires x > 0 ensures result == x { try { return x; } catch Failure { return 0; } } println(wrong(1));";
    let reference = run(source, true);
    assert_one(&reference); // Diagnostic-only runtime witness; this is not a typed acceptance oracle.
    let strict = run(source, false);
    assert!(!strict.status.success(), "{strict:?}");
    assert!(
        String::from_utf8_lossy(&strict.stderr).contains("Type check failed"),
        "{strict:?}"
    );
}
#[test]
fn pure_all_paths_contract_keeps_dead_catch_rejection() {
    let output = run(
        "fn fallback(int x) -> int requires x > 0 ensures result == x || result == 0 { try { return x; } catch Failure { return 0; } } println(fallback(1));",
        false,
    );
    assert!(!output.status.success(), "{output:?}");
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("does not correspond to any failure variant"),
        "{output:?}"
    );
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
        diagnostic.contains("violates `ensures") && diagnostic.contains("counterexample"),
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
    let (output, report) = run_captured(
        "fn risky(int x) -> int fails Failure { return x; } fn fallback(int x) -> int ensures result == x || result == 0 { try { return risky(x); } catch Failure { return 0; } } println(fallback(1));",
        false,
        true,
    );
    assert_one(&output);
    let report = report.unwrap_or_else(|| panic!("{output:?}"));
    use std::io::Write;
    std::io::stdout()
        .lock()
        .write_all(format!("TRY_FLOW_PUBLIC_REPORT {report}\n").as_bytes())
        .unwrap();
    let function = report["functions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|function| function["name"] == "fallback")
        .unwrap();
    let clause = function["clauses"]
        .as_array()
        .unwrap()
        .iter()
        .find(|clause| clause["kind"] == "ensures")
        .unwrap();
    assert_eq!(clause["verdict"], "unknown", "{report}");
    assert!(
        clause.get("smtlib2").is_none(),
        "unsupported call must not acquire implementation proof: {report}"
    );
}
