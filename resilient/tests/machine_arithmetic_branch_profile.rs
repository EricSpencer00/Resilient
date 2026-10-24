//! Independently audited branch inventory needs the actual compiler's BV sessions.
#![cfg(feature = "z3")]
#[test]
fn branch_identity_retains_two_standard_bv_sessions() {
    let root = std::env::temp_dir().join(format!("machine_branch_profile_{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let source = "fn identity(int x) -> int ensures result == x { if x >= 0 { return x; } else { return x; } }";
    std::fs::write(root.join("input.rz"), source).unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_rz"))
        .args(["--no-cache", "--emit-contract-certificate"])
        .arg(root.join("report.json"))
        .arg(root.join("input.rz"))
        .env_remove("RESILIENT_OVERFLOW_MODE")
        .output()
        .unwrap();
    let value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("report.json")).unwrap()).unwrap();
    std::fs::write(root.join("stdout.txt"), &output.stdout).unwrap();
    std::fs::write(root.join("stderr.txt"), &output.stderr).unwrap();
    std::fs::write(root.join("exit-status.txt"), output.status.to_string()).unwrap();
    if let Some(destination) = std::env::var_os("MACHINE_ARITHMETIC_RECORDS") {
        let destination = std::path::PathBuf::from(destination)
            .join("branch_identity_retains_two_standard_bv_sessions");
        std::fs::create_dir_all(&destination).unwrap();
        for name in [
            "input.rz",
            "report.json",
            "stdout.txt",
            "stderr.txt",
            "exit-status.txt",
        ] {
            std::fs::copy(root.join(name), destination.join(name)).unwrap();
        }
    }
    assert!(output.status.success(), "{output:?}");
    let queries = value["functions"][0]["implementation_queries"]
        .as_array()
        .unwrap();
    assert_eq!(queries.len(), 2, "{value}");
    for query in queries {
        assert_eq!(query["result"], "unsat", "{value}");
        let text = query["query"].as_str().unwrap();
        assert!(text.contains("BitVec 64"), "{text}");
        assert!(
            !text.contains("(model-"),
            "saved session must precede solver preprocessing: {text}"
        );
    }
    std::fs::remove_dir_all(root).unwrap();
}
