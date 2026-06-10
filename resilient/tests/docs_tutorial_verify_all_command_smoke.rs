//! RES-3217: Z3 tutorial uses the current `rz verify-all` command.

#[test]
fn z3_tutorial_uses_rz_verify_all_examples() {
    let doc = include_str!("../../docs/tutorial/05-verifying-with-z3.md");

    for expected in ["rz verify-all certs", "rz verify-all --z3 certs"] {
        assert!(
            doc.contains(expected),
            "Z3 tutorial missing current verify-all command {expected:?}"
        );
    }
    assert!(
        !doc.contains("resilient verify-all"),
        "Z3 tutorial should not use the retired `resilient` command"
    );
}
