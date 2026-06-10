//! RES-3257: self-host README labels lexer coverage without overstating it.

#[test]
fn self_host_readme_lexer_criterion_matches_partial_coverage() {
    let doc = include_str!("../../self-host/README.md");

    assert!(
        doc.contains("`self-host/lexer.rz` covers the curated lexer parity corpus"),
        "self-host README should label the current lexer coverage goal"
    );
    assert!(
        doc.contains(
            "full coverage matching every example in `resilient/examples/` is a follow-up"
        ),
        "self-host README should keep the remaining lexer coverage explicit"
    );
    assert!(
        !doc.contains("`self-host/lexer.rz` implements the complete Resilient lexer"),
        "self-host README should not claim complete lexer coverage"
    );
}
