//! Public certificate replay acceptance, independent of the SMT encoder.
#![cfg(all(feature = "z3", unix))]

use sha2::{Digest, Sha256};
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

struct Fixture(PathBuf);

impl Fixture {
    fn new(script: &str) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "res_4879_{}_{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&dir).unwrap();
        let query = b"(assert false)\n(check-sat)\n";
        std::fs::write(dir.join("proof.smt2"), query).unwrap();
        let manifest = serde_json::json!({
            "program": "example.rz",
            "obligations": [{"fn": "example", "kind": "ensures", "idx": 0,
                "cert": "proof.smt2", "sha256": format!("{:x}", Sha256::digest(query))}]
        });
        std::fs::write(dir.join("manifest.json"), manifest.to_string()).unwrap();
        let solver = dir.join("solver");
        std::fs::write(&solver, format!("#!/bin/sh\n{script}\n")).unwrap();
        std::fs::set_permissions(solver, std::fs::Permissions::from_mode(0o700)).unwrap();
        Self(dir)
    }

    fn check(&self, accepted: bool) {
        let output = Command::new(env!("CARGO_BIN_EXE_rz"))
            .arg("verify-all")
            .arg(&self.0)
            .arg("--z3")
            .env("Z3_BINARY", self.0.join("solver"))
            .output()
            .unwrap();
        assert_eq!(output.status.success(), accepted, "{output:?}");
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).contains("all checks passed"),
            accepted,
            "{output:?}"
        );
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn replay_accepts_one_clean_unsat_response() {
    Fixture::new("printf 'unsat\\n'").check(true);
}

#[test]
fn replay_rejects_unsat_from_failed_process() {
    Fixture::new("printf 'unsat\\n'; exit 7").check(false);
}

#[test]
fn replay_rejects_conflicting_or_extra_responses() {
    for response in [
        "sat",
        "unknown",
        "unsat",
        "(error bad-query)",
        "unsupported",
    ] {
        Fixture::new(&format!("printf 'unsat\\n{response}\\n'")).check(false);
    }
}

#[test]
fn replay_rejects_stderr_even_after_unsat() {
    Fixture::new("printf 'unsat\\n'; printf 'solver error\\n' >&2").check(false);
}

#[test]
fn replay_rejects_invalid_utf8_after_unsat() {
    Fixture::new("printf 'unsat\\n\\377'").check(false);
}

#[test]
fn replay_rejects_nonproof_responses() {
    for response in ["sat", "unknown", "", "(error bad-query)"] {
        Fixture::new(&format!("printf '{response}\\n'")).check(false);
    }
}
