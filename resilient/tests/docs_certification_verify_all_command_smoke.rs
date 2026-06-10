//! RES-3218: certification docs use the current `rz verify-all` command.

#[test]
fn certification_docs_use_rz_verify_all_examples() {
    let doc = include_str!("../../docs/certification.md");

    for expected in [
        "rz verify-all ./artifacts/certs",
        "rz verify-all ./artifacts/certs --z3",
    ] {
        assert!(
            doc.contains(expected),
            "certification docs missing current verify-all command {expected:?}"
        );
    }
    assert!(
        !doc.contains("resilient verify-all"),
        "certification docs should not use the retired `resilient` command"
    );
}
