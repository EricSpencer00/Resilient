//! RES-3233: E0010 docs use current verifier/audit wording.

#[test]
fn e0010_docs_use_rz_audit_for_z3_checking() {
    let doc = include_str!("../../docs/errors/E0010.md");

    assert!(
        doc.contains("`rz --audit` on a binary built with `--features z3`"),
        "E0010 docs should describe the current rz audit verifier path"
    );
    assert!(
        !doc.contains("`resilient verify`"),
        "E0010 docs should not mention the retired resilient verify command"
    );
}
