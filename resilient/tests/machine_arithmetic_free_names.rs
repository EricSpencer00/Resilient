//! The special postcondition name is not an implicit executable binding.
#![cfg(feature = "z3")]
#[test]
fn unbound_result_in_body_has_no_implementation_proof() {
    let root = std::env::temp_dir().join(format!("machine_free_name_{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("input.rz"),
        "fn f(int x) -> int ensures result == result { return result; }",
    )
    .unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_rz"))
        .args(["--no-cache", "--emit-contract-certificate"])
        .arg(root.join("report.json"))
        .arg(root.join("input.rz"))
        .env_remove("RESILIENT_OVERFLOW_MODE")
        .output()
        .unwrap();
    let value: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root.join("report.json")).unwrap_or_else(|_| panic!("{output:?}")),
    )
    .unwrap();
    let clause = &value["functions"][0]["clauses"][0];
    assert_ne!(clause["basis"], "implementation", "{value}");
    std::fs::remove_dir_all(root).unwrap();
}
