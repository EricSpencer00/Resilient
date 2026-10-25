//! A positive scalar-arm test must not hide wraparound in its wildcard arm.
#![cfg(feature = "z3")]
#[test]
fn wildcard_increment_is_refuted_at_signed_maximum() {
    let root = std::env::temp_dir().join(format!("machine_match_boundary_{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("input.rz"), "fn at_least_self(int x) -> int ensures result >= x { return match x { 0 => x, _ => x + 1 }; }").unwrap();
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
    assert_eq!(clause["verdict"], "fail", "{value}");
    assert!(
        clause["counterexample"]
            .as_str()
            .unwrap()
            .contains("9223372036854775807"),
        "{value}"
    );
    assert!(
        value["functions"][0]["implementation_queries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|q| q["result"] == "sat"),
        "{value}"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn guarded_wildcard_increment_retains_implementation_proof() {
    let root = std::env::temp_dir().join(format!("machine_guarded_match_{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("input.rz"), "fn at_least_self(int x) -> int requires x < 9223372036854775807 ensures result >= x { return match x { 0 => x, _ => x + 1 }; }").unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_rz"))
        .args(["--no-cache", "--emit-contract-certificate"])
        .arg(root.join("report.json"))
        .arg(root.join("input.rz"))
        .env_remove("RESILIENT_OVERFLOW_MODE")
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("report.json")).unwrap()).unwrap();
    let clause = &value["functions"][0]["clauses"][1];
    assert_eq!(clause["verdict"], "pass", "{value}");
    assert_eq!(clause["basis"], "implementation", "{value}");
    assert!(
        clause["smtlib2"].as_str().unwrap().contains("BitVec 64"),
        "{value}"
    );
    let queries = value["functions"][0]["implementation_queries"]
        .as_array()
        .unwrap();
    assert_eq!(queries.len(), 2, "{value}");
    assert!(
        queries.iter().all(|query| query["result"] == "unsat"),
        "{value}"
    );
    std::fs::remove_dir_all(root).unwrap();
}
