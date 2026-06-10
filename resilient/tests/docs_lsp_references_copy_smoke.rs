//! RES-3263: LSP docs describe implemented find-references support.

#[test]
fn lsp_docs_describe_find_references_support() {
    let docs = include_str!("../../docs/lsp.md");

    for expected in [
        "### Find references",
        "Running \"find references\" on a supported identifier returns LSP",
        "Top-level functions across the current file and imported workspace",
        "Struct types across the current file and imported workspace files.",
        "Same-file variable declarations, reads, and writes.",
        "standard `includeDeclaration` request flag",
    ] {
        assert!(
            docs.contains(expected),
            "LSP docs should describe find-references support; missing {expected:?}"
        );
    }
}
