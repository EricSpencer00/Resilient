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

// RES-4095 increment 4: `dyn Trait` in generic/container position.

#[test]
fn check_accepts_array_dyn_trait_heterogeneous_elements() {
    let output = Command::new(bin())
        .args(["check", "examples/trait_dyn_object_array_basic.rz"])
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
fn check_rejects_unknown_trait_in_array_dyn_annotation() {
    let output = Command::new(bin())
        .args([
            "check",
            "examples/trait_dyn_object_array_unknown_trait_reject.rz",
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
        stderr.contains("unknown trait `Frobnicate`"),
        "expected unknown-trait diagnostic; got: {stderr}"
    );
}

#[test]
fn check_rejects_not_object_safe_trait_in_array_dyn() {
    let output = Command::new(bin())
        .args(["check", "examples/trait_dyn_object_array_safety_reject.rz"])
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
        stderr.contains("[E0021]"),
        "expected object-safety diagnostic; got: {stderr}"
    );
}

#[test]
fn check_rejects_array_dyn_coercion_when_element_does_not_implement() {
    let output = Command::new(bin())
        .args([
            "check",
            "examples/trait_dyn_object_array_coercion_reject.rz",
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
        stderr.contains("does not implement `Shape`"),
        "expected coercion diagnostic; got: {stderr}"
    );
}

#[test]
fn check_rejects_struct_field_coercion_when_value_does_not_implement() {
    let output = Command::new(bin())
        .args([
            "check",
            "examples/trait_dyn_object_field_coercion_reject.rz",
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
        stderr.contains("does not implement `Shape`") && stderr.contains("field `inner`"),
        "expected field-coercion diagnostic; got: {stderr}"
    );
}

#[test]
fn check_rejects_return_type_coercion_when_value_does_not_implement() {
    let output = Command::new(bin())
        .args([
            "check",
            "examples/trait_dyn_object_return_coercion_reject.rz",
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
        stderr.contains("does not implement `Shape`") && stderr.contains("return value"),
        "expected return-coercion diagnostic; got: {stderr}"
    );
}

// RES-4095 increment 5: flow-sensitive coercion checking (issue item 4)
// — a `dyn`-typed slot fed through a local-variable alias chain, or a
// fn call whose declared return type is a concrete struct.

#[test]
fn check_accepts_dyn_trait_coercion_through_alias_chain() {
    let output = Command::new(bin())
        .args(["check", "examples/trait_dyn_flow_alias_basic.rz"])
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
fn check_rejects_dyn_trait_coercion_through_local_alias() {
    let output = Command::new(bin())
        .args(["check", "examples/trait_dyn_flow_alias_reject.rz"])
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
        "expected alias-coercion diagnostic; got: {stderr}"
    );
}

#[test]
fn check_rejects_dyn_trait_coercion_through_fn_return_type() {
    let output = Command::new(bin())
        .args(["check", "examples/trait_dyn_flow_return_reject.rz"])
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
        "expected return-type-coercion diagnostic; got: {stderr}"
    );
}
