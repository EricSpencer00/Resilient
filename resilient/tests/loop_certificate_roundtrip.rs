//! Generated loop proofs must survive the same signed replay used by consumers.
#![cfg(feature = "z3")]

use ed25519_dalek::SigningKey;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

struct Fixture(PathBuf);

impl Fixture {
    fn new(source: &str) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "res_loop_roundtrip_{}_{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("program.rz"), source).unwrap();
        let key = SigningKey::generate(&mut rand_core::OsRng);
        for (name, label, bytes) in [
            ("private.pem", "PRIVATE", key.to_bytes()),
            ("public.pem", "PUBLIC", key.verifying_key().to_bytes()),
        ] {
            let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
            std::fs::write(root.join(name), format!(
                "-----BEGIN ED25519 {label} KEY-----\n{hex}\n-----END ED25519 {label} KEY-----\n"
            )).unwrap();
        }
        Self(root)
    }

    fn emit(&self) -> std::process::Output {
        Command::new(env!("CARGO_BIN_EXE_rz"))
            .args(["--no-cache", "-t", "--emit-certificate"])
            .arg(self.0.join("certs"))
            .arg("--sign-cert")
            .arg(self.0.join("private.pem"))
            .arg(self.0.join("program.rz"))
            .output()
            .unwrap()
    }

    fn replay(&self, subproofs: usize) {
        let emitted = self.emit();
        assert!(emitted.status.success(), "emit: {emitted:?}");
        for command in ["verify-cert", "verify-all"] {
            let mut verify = Command::new(env!("CARGO_BIN_EXE_rz"));
            verify
                .arg(command)
                .arg(self.0.join("certs"))
                .arg("--pubkey")
                .arg(self.0.join("public.pem"));
            if command == "verify-all" {
                verify.arg("--z3").env("Z3_BINARY", "z3");
            }
            let output = verify.output().unwrap();
            assert!(output.status.success(), "{command}: {output:?}");
        }
        let manifest: serde_json::Value =
            serde_json::from_slice(&std::fs::read(self.0.join("certs/manifest.json")).unwrap())
                .unwrap();
        let obligations = manifest["obligations"].as_array().unwrap();
        assert_eq!(
            obligations.len(),
            subproofs,
            "all base and step proofs must be retained"
        );
        for obligation in obligations {
            let query = std::fs::read_to_string(
                self.0
                    .join("certs")
                    .join(obligation["cert"].as_str().unwrap()),
            )
            .unwrap();
            assert_eq!(query.matches("(check-sat)").count(), 1, "{query}");
            assert!(obligation["sig"].is_string());
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn while_invariant_certificates_replay_with_real_solver() {
    Fixture::new("let n = 0; while n < 3 invariant n >= 0 { n = n + 1; }").replay(2);
}

#[test]
fn finite_loop_certificates_replay_with_real_solver() {
    Fixture::new("let s = 0; for x in [1, 2, 3] invariant s >= 0 { s = s + x; }").replay(4);
}

#[test]
fn false_loop_invariant_does_not_emit_a_proof() {
    let fixture = Fixture::new("let n = 0; while n < 3 invariant n < 0 { n = n + 1; }");
    assert!(!fixture.emit().status.success());
    let manifest = fixture.0.join("certs/manifest.json");
    if manifest.exists() {
        let manifest: serde_json::Value =
            serde_json::from_slice(&std::fs::read(manifest).unwrap()).unwrap();
        assert!(manifest["obligations"].as_array().unwrap().is_empty());
    }
}
