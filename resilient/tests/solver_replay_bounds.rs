//! Replay resource limits are exercised through the public verifier command.
#![cfg(all(feature = "z3", unix))]

use sha2::{Digest, Sha256};
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

struct Fixture(PathBuf);

impl Fixture {
    fn new(script: &str, response: &[u8]) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "res_replay_bounds_{}_{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        let query = b"(assert false)\n(check-sat)\n";
        std::fs::write(root.join("proof.smt2"), query).unwrap();
        std::fs::write(root.join("response"), response).unwrap();
        std::fs::write(
            root.join("manifest.json"),
            serde_json::json!({"program": "f.rz", "obligations": [{
                "fn": "f", "kind": "ensures", "idx": 0, "cert": "proof.smt2",
                "sha256": format!("{:x}", Sha256::digest(query))
            }]})
            .to_string(),
        )
        .unwrap();
        let solver = root.join("solver");
        std::fs::write(&solver, format!("#!/bin/sh\n{script}\n")).unwrap();
        std::fs::set_permissions(solver, std::fs::Permissions::from_mode(0o700)).unwrap();
        Self(root)
    }

    fn check(&self, timeout: &str, accepted: bool) {
        let mut child = Command::new(env!("CARGO_BIN_EXE_rz"))
            .args(["verify-all", self.0.to_str().unwrap(), "--z3"])
            .env("Z3_BINARY", self.0.join("solver"))
            .env("RZ_SOLVER_TIMEOUT_MS", timeout)
            .env("REPLAY_RESPONSE", self.0.join("response"))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let start = Instant::now();
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            if start.elapsed() > Duration::from_secs(5) {
                let _ = child.kill();
                let _ = child.wait();
                panic!("verifier exceeded the independent harness watchdog");
            }
            std::thread::sleep(Duration::from_millis(5));
        };
        assert_eq!(
            status.success(),
            accepted,
            "timeout={timeout}, status={status}"
        );
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn replay_rejects_response_after_deadline() {
    Fixture::new("sleep 0.3; printf 'unsat\\n'", b"").check("50", false);
}

#[test]
fn replay_rejects_oversized_whitespace_after_unsat() {
    let mut response = b"unsat\n".to_vec();
    response.resize(131_072, b' ');
    Fixture::new("cat \"$REPLAY_RESPONSE\"", &response).check("2000", false);
}

#[test]
fn replay_rejects_invalid_timeout_configuration() {
    for timeout in ["0", "-1", "invalid", "300001"] {
        Fixture::new("printf 'unsat\\n'", b"").check(timeout, false);
    }
}

#[test]
fn replay_accepts_clean_response_within_budget() {
    Fixture::new("printf 'unsat\\n'", b"").check("2000", true);
}
