#![cfg(feature = "z3")]

use std::process::Command;

#[test]
fn stale_entry_state_never_becomes_a_cli_certificate() {
    let dir = std::env::temp_dir().join(format!("res_4901_certificate_e2e_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("scratch directory");
    let source = dir.join("sensor.rz");
    let certificates = dir.join("certificates");
    std::fs::write(
        &source,
        "let reading = 0; reading = -1; while reading < 1 invariant reading >= 0 { reading = reading + 1; }",
    ).expect("write sensor program");
    let output = Command::new(env!("CARGO_BIN_EXE_rz"))
        .args(["--no-cache", "--verbose", "-t", "--emit-certificate"])
        .arg(&certificates)
        .arg(&source)
        .output()
        .expect("run verifier CLI");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success(),
        "false invariant must fail at runtime"
    );
    assert!(stderr.contains("loop invariant violated"), "{stderr}");
    assert!(!stderr.contains("invariant proven"), "{stderr}");
    if certificates.exists() {
        for file in std::fs::read_dir(certificates).expect("read certificates") {
            let file = file.expect("certificate entry").path();
            if file
                .extension()
                .is_some_and(|extension| extension == "smt2")
            {
                let proof = std::fs::read_to_string(file).expect("read proof");
                assert!(
                    !proof.contains("loop-invariant proof certificate"),
                    "unsound proof: {proof}"
                );
            }
        }
    }
    std::fs::remove_dir_all(dir).expect("remove scratch directory");
}
