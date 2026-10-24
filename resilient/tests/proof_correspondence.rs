//! Source-authored inventory capture precedes strict source/query checkpoint review.
#![cfg(all(feature = "z3", unix))]

use ed25519_dalek::SigningKey;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::io::Write;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn capture(name: &str) {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let fixtures =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/proof_correspondence");
    let expectations_raw = std::fs::read(fixtures.join("expectations.json")).unwrap();
    let expectations: Value = serde_json::from_slice(&expectations_raw).unwrap();
    let fixture_name = format!("{name}.rz");
    let expected = expectations["fixtures"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["fixture"] == fixture_name)
        .unwrap();
    let source = std::fs::read(fixtures.join(&fixture_name)).unwrap();
    assert_eq!(hash(&source), expected["source_sha256"]);
    let root = std::env::temp_dir().join(format!(
        "correspondence_{}_{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir(&root).unwrap();
    std::fs::create_dir(root.join("inputs")).unwrap();
    std::fs::write(root.join("inputs/program.rz"), &source).unwrap();
    let key = SigningKey::generate(&mut rand_core::OsRng);
    for (filename, label, bytes) in [
        ("private.pem", "PRIVATE", key.to_bytes()),
        ("public.pem", "PUBLIC", key.verifying_key().to_bytes()),
    ] {
        let encoded: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
        std::fs::write(root.join(filename), format!("-----BEGIN ED25519 {label} KEY-----\n{encoded}\n-----END ED25519 {label} KEY-----\n")).unwrap();
    }
    let compiler = env!("CARGO_BIN_EXE_rz");
    let compilation = Command::new(compiler)
        .args(["--no-cache", "-t", "--emit-certificate"])
        .arg(root.join("certs"))
        .arg("--sign-cert")
        .arg(root.join("private.pem"))
        .arg(root.join("inputs/program.rz"))
        .output()
        .unwrap();
    assert!(
        compilation.status.success(),
        "fixture {name} parser/typechecker failed, not a correspondence result: {compilation:?}"
    );
    let manifest_raw = std::fs::read(root.join("certs/manifest.json")).unwrap();
    let manifest: Value = serde_json::from_slice(&manifest_raw).unwrap();
    let obligations = manifest["obligations"].as_array().unwrap();
    let mut queries = Vec::new();
    for obligation in obligations {
        let filename = obligation["cert"].as_str().unwrap();
        assert_eq!(Path::new(filename).file_name().unwrap(), filename);
        let bytes = std::fs::read(root.join("certs").join(filename)).unwrap();
        assert_eq!(hash(&bytes), obligation["sha256"]);
        queries.push(json!({"filename": filename, "kind": obligation["kind"], "sha256": hash(&bytes), "text": String::from_utf8(bytes).unwrap()}));
    }
    let replay = Command::new(compiler)
        .arg("verify-all")
        .arg(root.join("certs"))
        .arg("--source-root")
        .arg(root.join("inputs"))
        .arg("--pubkey")
        .arg(root.join("public.pem"))
        .arg("--z3")
        .output()
        .unwrap();
    let version = Command::new(compiler)
        .args(["--version", "--verbose"])
        .output()
        .unwrap();
    assert!(version.status.success(), "compiler identity command failed");
    let configured_solver = std::env::var("Z3_BINARY").unwrap_or_else(|_| "z3".into());
    let solver_lookup = Command::new("which")
        .arg(&configured_solver)
        .output()
        .unwrap();
    assert!(
        solver_lookup.status.success(),
        "configured solver unavailable"
    );
    let solver_path = String::from_utf8(solver_lookup.stdout).unwrap();
    let solver_path = solver_path.trim();
    let solver_version = Command::new(solver_path).arg("-version").output().unwrap();
    assert!(solver_version.status.success());
    let solver_identity = json!({"executable_sha256":hash(&std::fs::read(solver_path).unwrap()), "version":String::from_utf8_lossy(&solver_version.stdout), "scope":"executable identity only; complete dynamic linkage/environment not established"});
    let record = json!({"schema":"resilient-correspondence-capture-v1", "solver":solver_identity, "fixture":name, "claim":"source-authored path inventory and authenticated replay capture; structural correspondence not yet accepted", "expected":expected, "expectations_sha256":hash(&expectations_raw), "source":String::from_utf8(source).unwrap(), "compiler_sha256":hash(&std::fs::read(compiler).unwrap()), "compiler_version":String::from_utf8_lossy(&version.stdout), "manifest":manifest, "manifest_sha256":hash(&manifest_raw), "manifest_signature":std::fs::read_to_string(root.join("certs/manifest.sig")).unwrap(), "public_key":std::fs::read_to_string(root.join("public.pem")).unwrap(), "queries":queries, "replay_exit":replay.status.code(), "replay_stdout":String::from_utf8_lossy(&replay.stdout), "replay_stderr":String::from_utf8_lossy(&replay.stderr)});
    let line = serde_json::to_vec(&record).unwrap();
    assert!(
        line.len() < 256 * 1024,
        "public capture record exceeds bound"
    );
    // Direct writes preserve the public record in unchanged normal-CI job logs.
    let mut stdout = std::io::stdout().lock();
    stdout.write_all(b"CORRESPONDENCE_CAPTURE ").unwrap();
    stdout.write_all(&line).unwrap();
    stdout.write_all(b"\n").unwrap();
    drop(stdout);
    assert!(
        replay.status.success(),
        "source-authenticated replay failed: {replay:?}"
    );
    let implementation: Vec<_> = obligations
        .iter()
        .filter(|entry| entry["kind"] == "ensures_implementation")
        .collect();
    assert_eq!(
        implementation.len(),
        expected["paths"].as_array().unwrap().len(),
        "source-authored path inventory differs for {name}"
    );
    for query in record["queries"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|query| query["kind"] == "ensures_implementation")
    {
        assert!(
            query["text"]
                .as_str()
                .unwrap()
                .contains("; implementation-profile: vm-i64-wrap-v1"),
            "actual integrated machine-profile required; capture is not final acceptance"
        );
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn captures_straight_source_inventory() {
    capture("straight");
}
#[test]
fn captures_if_else_source_inventory() {
    capture("if_else");
}
#[test]
fn captures_early_fallthrough_source_inventory() {
    capture("early_fallthrough");
}
#[test]
fn captures_first_match_guarded_source_inventory() {
    capture("first_match_guarded");
}
#[test]
fn captures_try_conservative_model_inventory() {
    capture("try_overapprox");
}

fn capture_refutation(name: &str) {
    let fixtures =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/proof_correspondence");
    let expectations_raw = std::fs::read(fixtures.join("negative-expectations.json")).unwrap();
    let expectations: Value = serde_json::from_slice(&expectations_raw).unwrap();
    let filename = format!("negative_{name}.rz");
    let expected = expectations["fixtures"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["fixture"] == filename)
        .unwrap();
    let source = std::fs::read(fixtures.join(&filename)).unwrap();
    assert_eq!(hash(&source), expected["source_sha256"]);
    let compiler = env!("CARGO_BIN_EXE_rz");
    let result = Command::new(compiler)
        .args(["--no-cache", "-t"])
        .arg(fixtures.join(filename))
        .output()
        .unwrap();
    let diagnostic = format!(
        "{}{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    let version = Command::new(compiler)
        .args(["--version", "--verbose"])
        .output()
        .unwrap();
    assert!(version.status.success());
    let configured_solver = std::env::var("Z3_BINARY").unwrap_or_else(|_| "z3".into());
    let lookup = Command::new("which")
        .arg(configured_solver)
        .output()
        .unwrap();
    assert!(lookup.status.success());
    let solver_path = String::from_utf8(lookup.stdout).unwrap();
    let solver_path = solver_path.trim();
    let solver_version = Command::new(solver_path).arg("-version").output().unwrap();
    assert!(solver_version.status.success());
    let record = json!({"schema":"resilient-correspondence-refutation-capture-v1", "fixture":name, "claim":"source/model negative control only; try handler reachability and full correspondence not established", "expected":expected, "expectations_sha256":hash(&expectations_raw), "source":String::from_utf8(source).unwrap(), "compiler_sha256":hash(&std::fs::read(compiler).unwrap()), "compiler_version":String::from_utf8_lossy(&version.stdout), "solver":{"executable_sha256":hash(&std::fs::read(solver_path).unwrap()), "version":String::from_utf8_lossy(&solver_version.stdout), "scope":"executable only; complete linkage/environment not established"}, "exit":result.status.code(), "diagnostic":diagnostic});
    let raw = serde_json::to_vec(&record).unwrap();
    assert!(raw.len() < 256 * 1024, "emitted record bound only");
    let mut stdout = std::io::stdout().lock();
    stdout
        .write_all(b"CORRESPONDENCE_REFUTATION_CAPTURE ")
        .unwrap();
    stdout.write_all(&raw).unwrap();
    stdout.write_all(b"\n").unwrap();
    drop(stdout);
    assert!(
        !result.status.success(),
        "wrong return was accepted for {name}"
    );
    assert!(
        diagnostic.contains("fn `wrong` violates `ensures"),
        "generic syntax/type failure does not establish expected refutation: {diagnostic}"
    );
}

#[test]
fn captures_straight_wrong_return_refutation() {
    capture_refutation("straight");
}
#[test]
fn captures_else_wrong_return_refutation() {
    capture_refutation("if_else");
}
#[test]
fn captures_fallthrough_wrong_return_refutation() {
    capture_refutation("early_fallthrough");
}
#[test]
fn captures_match_arm_wrong_return_refutation() {
    capture_refutation("first_match_guarded");
}
#[test]
fn captures_try_wrong_handler_conservative_refutation() {
    capture_refutation("try_overapprox");
}
