//! Iteration-binding invariants remain runtime checked until fully modeled.
#![cfg(feature = "z3")]

use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

fn no_certificate(source: &str, succeeds: bool) {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let root = std::env::temp_dir().join(format!(
        "res_finite_guard_{}_{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("program.rz"), source).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_rz"))
        .args(["--no-cache", "-t", "--emit-certificate"])
        .arg(root.join("certs"))
        .arg(root.join("program.rz"))
        .output()
        .unwrap();
    let manifest = root.join("certs/manifest.json");
    let obligations = if manifest.exists() {
        let value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(manifest).unwrap()).unwrap();
        value["obligations"].as_array().unwrap().len()
    } else {
        0
    };
    std::fs::remove_dir_all(root).unwrap();
    assert_eq!(output.status.success(), succeeds, "{output:?}");
    assert_eq!(
        obligations, 0,
        "unsupported iteration semantics must not receive a proof"
    );
}

#[test]
fn shadowed_iteration_binding_is_not_certified() {
    no_certificate("let x = 1; for x in [-1] invariant x >= 0 { }", false);
}

#[test]
fn changing_iteration_binding_is_not_certified() {
    no_certificate("let x = 1; for x in [1, -1] invariant x >= 0 { }", false);
}

#[test]
fn valid_iteration_binding_retains_runtime_check() {
    no_certificate("let x = 1; for x in [1, 2] invariant x >= 0 { }", true);
}

#[test]
fn empty_iteration_does_not_claim_an_executed_invariant_proof() {
    no_certificate(
        "let s = 0; for x in [] invariant s >= 0 { s = s + x; }",
        true,
    );
}
