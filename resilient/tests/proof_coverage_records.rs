//! Preserve observed defects separately from absent-feature baseline attestations.
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::Path;
use std::process::Command;

#[test]
fn retained_old_auditor_defects_preserve_original_bytes_and_outcomes() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let records = repo.join("qualification/proof-coverage/reviews/old-helper-40065786584");
    let sums = std::fs::read_to_string(records.join("SHA256SUMS")).unwrap();
    for line in sums.lines() {
        let (expected, name) = line.split_once("  ").unwrap();
        assert!(Path::new(name).is_relative());
        assert!(
            Path::new(name)
                .components()
                .all(|part| matches!(part, std::path::Component::Normal(_)))
        );
        let bytes = std::fs::read(records.join(name)).unwrap();
        let actual: String = Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        assert_eq!(actual, expected, "retained file {name}");
    }
    let index: Value =
        serde_json::from_slice(&std::fs::read(records.join("index.json")).unwrap()).unwrap();
    let cases = index["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 7);
    for case in cases {
        let name = case.as_str().unwrap();
        let bundle = records.join(name);
        let result = Command::new("python3")
            .arg(repo.join("agent-scripts/e2e-evidence.py"))
            .arg("verify")
            .arg(&bundle)
            .output()
            .unwrap();
        assert!(result.status.success(), "{name}: {result:?}");
        let manifest: Value =
            serde_json::from_slice(&std::fs::read(bundle.join("manifest.json")).unwrap()).unwrap();
        assert_eq!(manifest["source"]["commit"], index["source_commit"]);
        assert_eq!(manifest["result"]["status"], "failed");
        assert!(
            manifest["result"]["outcomes"]
                .as_array()
                .unwrap()
                .iter()
                .any(|outcome| outcome["name"] == name && outcome["status"] == "failed")
        );
        assert_eq!(
            manifest["inputs"][0]["sha256"],
            "fb6784849f97b759565deec3ad726d3fb550dc180066891b205e0383c8a35a03"
        );
        let stdout = std::fs::read_to_string(records.join(format!("{name}.stdout"))).unwrap();
        assert!(
            stdout.contains(name) && stdout.contains("FAILED"),
            "named observed failure missing: {name}"
        );
    }
}
