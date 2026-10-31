//! Computed callable provenance must preserve the refinement extern guard.
use std::process::Command;

#[test]
fn computed_targets_keep_extern_contract_obligations() {
    let root = std::env::temp_dir().join(format!("refines-computed-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("counter.tla"),
        "---- MODULE counter ----\nInc == TRUE\n====\n",
    )
    .unwrap();
    let cases: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/refines_computed_targets.json")).unwrap();
    for case in cases.as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let source = case["source"].as_str().unwrap();
        let input = root.join(format!("{name}.rz"));
        std::fs::write(&input, source).unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_rz"))
            .args(["--typecheck", "--no-cache"])
            .arg(&input)
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        if case["reject"].as_bool().unwrap() {
            assert!(
                !output.status.success(),
                "{name}: uncontracted target accepted"
            );
            assert!(
                stderr.contains("error[refinement]") && stderr.contains("extern fn `foreign`"),
                "{name}: {stderr}"
            );
        } else {
            assert!(output.status.success(), "{name}: {stderr}");
        }
    }
    std::fs::remove_dir_all(root).unwrap();
}
