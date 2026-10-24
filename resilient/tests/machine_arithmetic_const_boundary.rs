//! Constant boundaries must reach conservative proof handling without a panic.
#![cfg(feature = "z3")]
#[test]
fn minimum_negation_typecheck_does_not_panic() {
    let root = std::env::temp_dir().join(format!("machine_const_boundary_{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("input.rz"), "fn f(int x) -> int requires -(-9223372036854775807 - 1) == (-9223372036854775807 - 1) { return x; }").unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_rz"))
        .args(["--no-cache", "-t", "--emit-contract-certificate"])
        .arg(root.join("report.json"))
        .arg(root.join("input.rz"))
        .env_remove("RESILIENT_OVERFLOW_MODE")
        .output()
        .unwrap();
    assert!(
        !String::from_utf8_lossy(&output.stderr).contains("panicked"),
        "{output:?}"
    );
    assert!(root.join("report.json").is_file(), "{output:?}");
    std::fs::remove_dir_all(root).unwrap();
}
