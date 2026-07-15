//! A-E3 (RES-3933): CLI-level smoke tests for associated-type
//! projection resolution — accept + reject cases, mirroring the
//! `check_smoke.rs` pattern used for other typecheck diagnostics.

use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rz")
}

#[test]
fn check_accepts_valid_self_assoc_projection() {
    let output = Command::new(bin())
        .args(["check", "examples/trait_associated_type_projection.rz"])
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
fn check_rejects_unknown_associated_type_binding() {
    let output = Command::new(bin())
        .args([
            "check",
            "examples/trait_associated_type_unknown_binding_reject.rz",
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
        stderr.contains("binds unknown associated type `Voltage`"),
        "expected unknown-binding diagnostic; got: {stderr}"
    );
    assert!(
        stderr.contains("does not declare it"),
        "expected unknown-binding diagnostic; got: {stderr}"
    );
}

#[test]
fn check_rejects_self_assoc_return_type_mismatch() {
    let output = Command::new(bin())
        .args([
            "check",
            "examples/trait_associated_type_return_mismatch_reject.rz",
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
        stderr.contains("return type mismatch"),
        "expected return-type-mismatch diagnostic; got: {stderr}"
    );
    // A-E3: the diagnostic should show the *resolved* concrete type
    // (`int`, from this impl's `type Width = int;` binding), not the
    // raw, unresolved `Self::Width` projection text — proof the
    // projection actually participated in type checking rather than
    // being compared as an opaque unresolvable name.
    assert!(
        stderr.contains("declared int"),
        "expected the resolved concrete type in the diagnostic; got: {stderr}"
    );
}

// --- A-E3 follow-up (#4067): generic-context projections + let bindings ---

#[test]
fn check_accepts_param_and_return_position_generic_projections() {
    let output = Command::new(bin())
        .args(["check", "examples/trait_associated_type_param_position.rz"])
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
fn check_rejects_generic_projection_of_undeclared_assoc_type() {
    let output = Command::new(bin())
        .args([
            "check",
            "examples/trait_associated_type_generic_projection_reject.rz",
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
        stderr.contains("projects `T::Bogus`"),
        "expected generic-projection diagnostic; got: {stderr}"
    );
    assert!(
        stderr.contains("declares associated type `Bogus`"),
        "expected generic-projection diagnostic; got: {stderr}"
    );
}

#[test]
fn check_accepts_self_assoc_let_binding_annotation() {
    let output = Command::new(bin())
        .args(["check", "examples/trait_associated_type_let_binding.rz"])
        .output()
        .expect("spawn resilient check");
    assert_eq!(
        output.status.code(),
        Some(0),
        "expected exit 0; stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
}
