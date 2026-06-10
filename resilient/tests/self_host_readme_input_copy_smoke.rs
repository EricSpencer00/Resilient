//! RES-3255: self-host README documents the current lexer input path.

#[test]
fn self_host_readme_uses_self_host_input_in_acceptance_table() {
    let doc = include_str!("../../self-host/README.md");

    assert!(
        doc.contains("`SELF_HOST_INPUT=<input_file> rz self-host/lexer.rz`"),
        "self-host README should label the supported env-var input path"
    );
    assert!(
        doc.contains("env-var input is the current language-level substitute"),
        "self-host README should explain why SELF_HOST_INPUT is used"
    );
    assert!(
        !doc.contains("resilient run self-host/lexer.rz -- <input_file>"),
        "self-host README should not document the retired resilient run shape"
    );
}
