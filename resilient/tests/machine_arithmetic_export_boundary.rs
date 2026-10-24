//! Implementation-grounded contract proofs must survive the same signed replay used by consumers.
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
            "res_machine_global_{}_{}",
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
                verify
                    .arg("--require-manifest-signature")
                    .arg("--z3")
                    .env("Z3_BINARY", "z3");
            }
            let output = verify.output().unwrap();
            assert!(output.status.success(), "{command}: {output:?}");
        }
        let manifest: serde_json::Value =
            serde_json::from_slice(&std::fs::read(self.0.join("certs/manifest.json")).unwrap())
                .unwrap();
        let obligations: Vec<_> = manifest["obligations"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|o| o["kind"] == "ensures_implementation")
            .collect();
        assert_eq!(
            obligations.len(),
            subproofs,
            "every implementation path must be retained"
        );
        for obligation in obligations {
            let query = std::fs::read_to_string(
                self.0
                    .join("certs")
                    .join(obligation["cert"].as_str().unwrap()),
            )
            .unwrap();
            assert_eq!(query.matches("(check-sat)").count(), 1, "{query}");
            assert!(query.contains("BitVec 64"), "{query}");
            assert!(
                query.contains("implementation-profile: vm-i64-wrap-v1"),
                "{query}"
            );
            assert!(obligation["sig"].is_string());
            assert!(
                obligation["fn"].as_str().unwrap().starts_with("good:"),
                "{obligation}"
            );
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn global_float_is_not_exported_as_integer_implementation() {
    Fixture::new("let nan = 0.0 / 0.0; fn bad(int x) -> int ensures nan == nan && result == 0 { return 0; } fn good(int x) -> int ensures result == x { return x; }").replay(1);
}
