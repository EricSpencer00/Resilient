//! RES-3333: generic syntax docs use public inference terminology.

#[test]
fn syntax_docs_describe_generics_without_scaffolding_jargon() {
    let syntax = include_str!("../../docs/syntax.md");

    for expected in [
        "Type parameters currently have parser/AST support plus call-site\nmonomorphization.",
        "The typechecker builds on the Hindley-Milner\ninference machinery from RES-122.",
        "Constraints (`fn<T: Trait>`)\nare a future extension.",
    ] {
        assert!(
            syntax.contains(expected),
            "generic syntax docs should use clear public wording: {expected:?}"
        );
    }

    for stale in ["HM scaffolding", "parse-and-AST only"] {
        assert!(
            !syntax.contains(stale),
            "generic syntax docs should not retain stale/internal wording: {stale:?}"
        );
    }
}
