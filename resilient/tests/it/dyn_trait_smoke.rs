//! RES-4068 (A-E3 follow-up): CLI-level smoke tests for `dyn Trait`
//! trait-object type-checking — accept + reject cases, mirroring the
//! `associated_types_smoke.rs` pattern.

use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rz")
}

#[test]
fn check_accepts_dyn_trait_coercion_and_method_call() {
    let output = Command::new(bin())
        .args(["check", "examples/trait_dyn_object_basic.rz"])
        .output()
        .expect("spawn resilient check");
    assert_eq!(
        output.status.code(),
        Some(0),
        "expected exit 0; stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn check_rejects_unknown_trait_in_dyn_annotation() {
    let output = Command::new(bin())
        .args(["check", "examples/trait_dyn_object_unknown_trait_reject.rz"])
        .output()
        .expect("spawn resilient check");
    assert_eq!(
        output.status.code(),
        Some(1),
        "expected exit 1; stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("unknown trait `Frobnicate`"),
        "expected unknown-trait diagnostic; got: {stderr}"
    );
}

#[test]
fn check_rejects_dyn_trait_coercion_when_struct_does_not_implement() {
    let output = Command::new(bin())
        .args(["check", "examples/trait_dyn_object_coercion_reject.rz"])
        .output()
        .expect("spawn resilient check");
    assert_eq!(
        output.status.code(),
        Some(1),
        "expected exit 1; stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("type `Square` does not implement `Shape`"),
        "expected coercion diagnostic; got: {stderr}"
    );
}

// RES-4095 (dyn v2 increment 1): object-safety checking.

#[test]
fn check_rejects_dyn_trait_with_no_self_method() {
    let output = Command::new(bin())
        .args([
            "check",
            "examples/trait_dyn_object_safety_no_self_reject.rz",
        ])
        .output()
        .expect("spawn resilient check");
    assert_eq!(
        output.status.code(),
        Some(1),
        "expected exit 1; stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("[E0021]") && stderr.contains("not object-safe"),
        "expected object-safety diagnostic; got: {stderr}"
    );
}

#[test]
fn check_rejects_dyn_trait_with_self_returning_method() {
    let output = Command::new(bin())
        .args([
            "check",
            "examples/trait_dyn_object_safety_self_return_reject.rz",
        ])
        .output()
        .expect("spawn resilient check");
    assert_eq!(
        output.status.code(),
        Some(1),
        "expected exit 1; stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("[E0021]") && stderr.contains("returns `Self`"),
        "expected object-safety diagnostic; got: {stderr}"
    );
}
