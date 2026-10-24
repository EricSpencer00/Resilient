//! A rejected program may produce diagnostics, never an accepted proof package.
#![cfg(feature = "z3")]
#[test]
fn illtyped_neighbor_marks_report_diagnostic_only() {
    let root = std::env::temp_dir().join(format!("machine_rejected_report_{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("input.rz"), "fn good(int x) -> int ensures result == x { return x; } let bad: int = \"not an integer\";").unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_rz"))
        .args([
            "--no-cache",
            "--typecheck-strict",
            "--emit-contract-certificate",
        ])
        .arg(root.join("report.json"))
        .arg(root.join("input.rz"))
        .env_remove("RESILIENT_OVERFLOW_MODE")
        .output()
        .unwrap();
    assert!(!output.status.success(), "{output:?}");
    let report: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("report.json")).unwrap()).unwrap();
    assert_eq!(report["typecheck_status"], "failed", "{report}");
    assert_eq!(report["diagnostic_only"], true, "{report}");
    for function in report["functions"].as_array().unwrap() {
        for clause in function["clauses"].as_array().unwrap() {
            assert_ne!(clause["verdict"], "pass", "{report}");
            assert!(clause.get("smtlib2").is_none(), "{report}");
        }
    }
    std::fs::remove_dir_all(root).unwrap();
}
