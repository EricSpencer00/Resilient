//! RES-3251: self-host README treats parser.rz as a current artifact.

#[test]
fn self_host_readme_lists_parser_as_current_artifact() {
    let doc = include_str!("../../self-host/README.md");

    assert!(
        doc.contains("Three artifact generations live here"),
        "self-host README should describe the current artifact count"
    );
    assert!(
        doc.contains("`parser.rz` + `parser_tests/` + `parity_corpus/`"),
        "self-host README should list parser.rz in the artifact table"
    );
    assert!(
        doc.contains("`parser.rz` exists and is covered by"),
        "self-host README should frame parser work as present but expanding"
    );
    assert!(
        !doc.contains("Two artifacts live here"),
        "self-host README should not say only two artifacts exist"
    );
    assert!(
        !doc.contains("Self-hosting parser** is tracked as the follow-up"),
        "self-host README should not describe parser.rz as future-only"
    );
}
