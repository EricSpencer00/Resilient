//! RES-3367: certification roadmap docs state capability boundaries plainly.

#[test]
fn certification_docs_roadmap_avoids_current_capability_claims() {
    let doc = include_str!("../../docs/certification.md");

    for expected in [
        "The items below are roadmap work only: they are not",
        "current toolchain capability, certification evidence, or a claim of",
        "conformance.",
    ] {
        assert!(
            doc.contains(expected),
            "certification roadmap should state capability boundaries: {expected:?}"
        );
    }

    assert!(
        !doc.contains("Items below are planned, not delivered."),
        "certification roadmap should not use vague planned/not-delivered wording"
    );
}
