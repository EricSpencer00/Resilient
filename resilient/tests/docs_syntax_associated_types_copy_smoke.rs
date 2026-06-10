//! RES-3379: root syntax docs do not describe associated types as future work.

#[test]
fn root_syntax_docs_point_to_current_associated_type_section() {
    let syntax = include_str!("../../SYNTAX.md");

    for expected in [
        "Method names must be unique within a trait. Associated type members are\ndocumented later in this trait section.",
        "### Associated Types (RES-783)",
        "Traits can declare associated types",
    ] {
        assert!(
            syntax.contains(expected),
            "root syntax docs should point to current associated-type docs: {expected:?}"
        );
    }

    assert!(
        !syntax.contains("RES-779 (future) will add associated\ntypes."),
        "root syntax docs should not describe associated types as future work"
    );
}
