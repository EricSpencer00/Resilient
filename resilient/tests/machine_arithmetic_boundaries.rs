//! Additional failure witnesses and direct bytecode-VM checkpoints.
#![cfg(feature = "z3")]
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
fn run(case: &str, source: &str, vm: bool) -> (serde_json::Value, String) {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let root = std::env::temp_dir().join(format!(
        "machine_boundary_{}_{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("input.rz"), source).unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_rz"));
    command
        .args(["--no-cache", "--emit-contract-certificate"])
        .arg(root.join("report.json"))
        .arg(root.join("input.rz"))
        .env_remove("RESILIENT_OVERFLOW_MODE");
    if vm {
        command.arg("--vm");
    }
    let output = command.output().unwrap();
    let value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("report.json")).unwrap()).unwrap();
    std::fs::write(root.join("stdout.txt"), &output.stdout).unwrap();
    std::fs::write(root.join("stderr.txt"), &output.stderr).unwrap();
    if let Some(destination) = std::env::var_os("MACHINE_ARITHMETIC_RECORDS") {
        let destination = std::path::PathBuf::from(destination).join(case);
        std::fs::create_dir_all(&destination).unwrap();
        for name in ["input.rz", "report.json", "stdout.txt", "stderr.txt"] {
            std::fs::copy(root.join(name), destination.join(name)).unwrap();
        }
    }
    std::fs::remove_dir_all(root).unwrap();
    (value, String::from_utf8(output.stdout).unwrap())
}
#[test]
fn overflow_refutation_retains_sat_query_and_maximum_witness() {
    let (value, _) = run(
        "overflow_refutation_retains_sat_query_and_maximum_witness",
        "fn f(int x) -> int requires x >= 0 ensures result >= 0 { return x + 1; }",
        false,
    );
    let function = &value["functions"][0];
    let clause = &function["clauses"][1];
    assert_eq!(clause["verdict"], "fail", "{value}");
    assert!(
        clause["counterexample"]
            .as_str()
            .unwrap()
            .contains("9223372036854775807"),
        "{value}"
    );
    assert_eq!(
        function["implementation_queries"][0]["result"], "sat",
        "{value}"
    );
    assert!(
        function["implementation_queries"][0]["query"]
            .as_str()
            .unwrap()
            .contains("BitVec 64")
    );
}
#[test]
fn negation_refutation_retains_minimum_witness() {
    let (value, _) = run(
        "negation_refutation_retains_minimum_witness",
        "fn f(int x) -> int requires x < 0 ensures result > 0 { return -x; }",
        false,
    );
    let clause = &value["functions"][0]["clauses"][1];
    assert_eq!(clause["verdict"], "fail", "{value}");
    assert!(
        clause["counterexample"]
            .as_str()
            .unwrap()
            .contains("-9223372036854775808"),
        "{value}"
    );
}
#[test]
fn global_float_cannot_receive_integer_implementation_credit() {
    let (value, _) = run(
        "global_float_cannot_receive_integer_implementation_credit",
        "let nan = 0.0 / 0.0; fn f(int x) -> int ensures nan == nan && result == 0 { return 0; }",
        false,
    );
    let clause = &value["functions"][0]["clauses"][0];
    assert_eq!(clause["verdict"], "unknown", "{value}");
    assert_ne!(clause["basis"], "implementation", "{value}");
}
#[test]
fn conversion_is_outside_the_machine_profile() {
    let (value, _) = run(
        "conversion_is_outside_the_machine_profile",
        "fn f(int x) -> int ensures result == x { return as_int32(x); }",
        false,
    );
    assert_ne!(
        value["functions"][0]["clauses"][0]["basis"], "implementation",
        "{value}"
    );
}
#[test]
fn bytecode_vm_default_wrap_matches_boundary_checkpoints() {
    let (_, output) = run(
        "bytecode_vm_default_wrap_matches_boundary_checkpoints",
        "let maximum = 9223372036854775807; let minimum = -9223372036854775807 - 1; println(maximum + 1); println(-minimum); println(-3 / 2); println(minimum / -1); println(minimum % -1);",
        true,
    );
    let numbers: Vec<_> = output
        .lines()
        .filter(|line| line.parse::<i64>().is_ok())
        .collect();
    assert_eq!(
        numbers,
        [
            "-9223372036854775808",
            "-9223372036854775808",
            "-1",
            "-9223372036854775808",
            "0"
        ],
        "{output}"
    );
}
