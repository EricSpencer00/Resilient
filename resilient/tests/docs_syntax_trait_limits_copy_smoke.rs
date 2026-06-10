//! RES-3375: root syntax docs describe trait limitations with support boundaries.

#[test]
fn root_syntax_docs_describe_trait_limitations_without_future_labels() {
    let syntax = include_str!("../../SYNTAX.md");

    for expected in [
        "Projection syntax (`T::AssocType`) in generic bounds (RES-779 follow-up)",
        "`dyn Trait` / virtual tables (RES-293)",
        "Generic associated types are not supported yet",
        "Default method bodies are not supported yet",
        "Blanket impls and specialization are not supported yet",
    ] {
        assert!(
            syntax.contains(expected),
            "root syntax docs should describe trait limitation boundaries: {expected:?}"
        );
    }

    for stale in [
        "Generic associated types (future)",
        "Default method bodies (future)",
        "Blanket impls or specialization (future)",
    ] {
        assert!(
            !syntax.contains(stale),
            "root syntax docs should not use bare future labels: {stale:?}"
        );
    }
}
