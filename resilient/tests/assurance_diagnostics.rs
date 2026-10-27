//! Advisory scores describe observed signals without inferring authorship.

use std::{fs, process::Command};

#[test]
fn missing_signals_are_reported_without_authorship_or_proof_claims() {
    let directory = std::env::temp_dir().join(format!("res5100-{}", std::process::id()));
    fs::create_dir_all(&directory).unwrap();
    let source = directory.join("signals.rz");
    fs::write(&source, "fn unused(int x) -> int { return x; }\n").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_rz"))
        .arg("check")
        .arg(&source)
        .output()
        .unwrap();
    fs::remove_dir_all(&directory).unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stderr}");
    assert!(stderr.contains("warning[resilience]"), "{stderr}");
    assert!(stderr.contains("F — few annotation signals"), "{stderr}");
    assert!(stderr.contains("warning[vibe_debt]"), "{stderr}");
    assert!(stderr.contains("no tracked assurance signals"), "{stderr}");
    for unsupported in ["vibe-coded", "formally guaranteed", "fully verified"] {
        assert!(!stderr.contains(unsupported), "{stderr}");
    }
}
