//! RES-3373: tooling docs describe formatter comment preservation plainly.

#[test]
fn tooling_docs_describe_formatter_comment_preservation_boundary() {
    let docs = include_str!("../../docs/tooling.md");

    for expected in [
        "The formatter is a structural round-trip,\nand the parser discards comments.",
        "Comments are not preserved today.",
        "Run `fmt` only on code you're willing to re-attach comments to by",
        "hand; comment-preserving formatting is not available yet.",
    ] {
        assert!(
            docs.contains(expected),
            "tooling docs should describe formatter comment preservation boundaries: {expected:?}"
        );
    }

    assert!(
        !docs.contains("Comment-aware formatting is the next planned formatter\nimprovement."),
        "tooling docs should not use roadmap-ish comment-aware formatter wording"
    );
}
