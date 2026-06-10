//! RES-3365: structural enforcement docs state citation validation boundaries.

#[test]
fn structural_enforcement_docs_describe_citation_validation_boundary() {
    let docs = include_str!("../../docs/STRUCTURAL_ENFORCEMENT.md");

    for expected in [
        "**Citation validation boundary**: today L0012 requires the source",
        "a later hardening pass should also validate that the cited",
        "source resolves (URL, ISBN, repo path) and promote unresolved",
        "citations to a hard gate for safety-critical builds.",
    ] {
        assert!(
            docs.contains(expected),
            "structural enforcement docs should describe citation validation boundaries: {expected:?}"
        );
    }

    for stale in [
        "**Future work**: validate that the cited source actually exists",
        "promote the lint to a hard gate",
    ] {
        assert!(
            !docs.contains(stale),
            "structural enforcement docs should not use vague future-work wording: {stale:?}"
        );
    }
}
