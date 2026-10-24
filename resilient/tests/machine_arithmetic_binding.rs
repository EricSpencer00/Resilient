//! Machine arithmetic credit is tested through the shipped CLI, never a private prover API.
#![cfg(feature = "z3")]
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

fn report(case: &str, source: &str, mode: &str) -> serde_json::Value {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let root = std::env::temp_dir().join(format!(
        "machine_arithmetic_{}_{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("input.rz"), source).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_rz"))
        .args(["--no-cache", "-t", "--emit-contract-certificate"])
        .arg(root.join("report.json"))
        .arg(root.join("input.rz"))
        .env("RESILIENT_OVERFLOW_MODE", mode)
        .output()
        .unwrap();
    std::fs::write(root.join("stdout.txt"), &output.stdout).unwrap();
    std::fs::write(root.join("stderr.txt"), &output.stderr).unwrap();
    let bytes = std::fs::read(root.join("report.json"))
        .unwrap_or_else(|error| panic!("missing public report: {error}; {output:?}"));
    let value = serde_json::from_slice(&bytes).unwrap();
    if let Some(destination) = std::env::var_os("MACHINE_ARITHMETIC_RECORDS") {
        let destination = PathBuf::from(destination).join(case);
        std::fs::create_dir_all(&destination).unwrap();
        for name in ["input.rz", "report.json", "stdout.txt", "stderr.txt"] {
            std::fs::copy(root.join(name), destination.join(name)).unwrap();
        }
    }
    std::fs::remove_dir_all(root).unwrap();
    value
}
fn ensures(value: &serde_json::Value) -> &serde_json::Value {
    value["functions"][0]["clauses"]
        .as_array()
        .unwrap()
        .iter()
        .find(|clause| clause["kind"] == "ensures")
        .unwrap()
}
fn machine_pass(case: &str, source: &str) {
    let value = report(case, source, "wrap");
    let clause = ensures(&value);
    assert_eq!(clause["basis"], "implementation", "{value}");
    assert_eq!(clause["verdict"], "pass", "{value}");
    let query = clause["smtlib2"].as_str().unwrap();
    assert!(
        query.contains("BitVec 64"),
        "implementation query must encode i64: {query}"
    );
    assert_eq!(value["implementation_profile"], "vm-i64-wrap-v1", "{value}");
}
fn withheld(case: &str, source: &str, mode: &str) {
    let value = report(case, source, mode);
    let clause = ensures(&value);
    assert!(
        !(clause["basis"] == "implementation" && clause["verdict"] == "pass"),
        "{value}"
    );
    assert!(
        clause.get("smtlib2").is_none(),
        "unsupported implementation must not export proof: {value}"
    );
}
#[test]
fn overflow_addition_is_not_proven_nonnegative() {
    withheld(
        "overflow_addition_is_not_proven_nonnegative",
        "fn f(int x) -> int requires x >= 0 ensures result >= 0 { return x + 1; }",
        "wrap",
    );
}
#[test]
fn minimum_negation_is_not_proven_positive() {
    withheld(
        "minimum_negation_is_not_proven_positive",
        "fn f(int x) -> int requires x < 0 ensures result > 0 { return -x; }",
        "wrap",
    );
}
#[test]
fn bounded_addition_has_machine_proof() {
    machine_pass(
        "bounded_addition_has_machine_proof",
        "fn f(int x) -> int requires x >= 0 requires x < 100 ensures result > x { return x + 1; }",
    );
}
#[test]
fn maximum_increment_wraps_to_minimum() {
    machine_pass(
        "maximum_increment_wraps_to_minimum",
        "fn f(int x) -> int requires x == 9223372036854775807 ensures result == (-9223372036854775807 - 1) { return x + 1; }",
    );
}
#[test]
fn negative_division_truncates_toward_zero() {
    machine_pass(
        "negative_division_truncates_toward_zero",
        "fn f(int x) -> int requires x == -3 ensures result == -1 { return x / 2; }",
    );
}
#[test]
fn minimum_division_wraps() {
    machine_pass(
        "minimum_division_wraps",
        "fn f(int x) -> int requires x == (-9223372036854775807 - 1) ensures result == x { return x / -1; }",
    );
}
#[test]
fn minimum_remainder_is_zero() {
    machine_pass(
        "minimum_remainder_is_zero",
        "fn f(int x) -> int requires x == (-9223372036854775807 - 1) ensures result == 0 { return x % -1; }",
    );
}
#[test]
fn division_by_zero_has_no_implementation_credit() {
    withheld(
        "division_by_zero_has_no_implementation_credit",
        "fn f(int x) -> int ensures result == result { return x / 0; }",
        "wrap",
    );
}
#[test]
fn saturating_mode_has_no_wrap_credit() {
    withheld(
        "saturating_mode_has_no_wrap_credit",
        "fn f(int x) -> int ensures result == x + 1 { return x + 1; }",
        "saturate",
    );
}
#[test]
fn trapping_mode_has_no_wrap_credit() {
    withheld(
        "trapping_mode_has_no_wrap_credit",
        "fn f(int x) -> int ensures result == x + 1 { return x + 1; }",
        "trap",
    );
}
#[test]
fn narrow_width_has_no_i64_credit() {
    withheld(
        "narrow_width_has_no_i64_credit",
        "fn f(i32 x) -> i32 ensures result == x + 1 { return x + 1; }",
        "wrap",
    );
}
#[test]
fn declared_contract_behavior_remains_available() {
    let value = report(
        "declared_contract_behavior_remains_available",
        "fn f(int x) -> int requires x + 0 == x { return x; }",
        "wrap",
    );
    assert_eq!(
        value["functions"][0]["clauses"][0]["verdict"], "pass",
        "{value}"
    );
}
