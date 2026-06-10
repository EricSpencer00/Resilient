//! RES-3267: LSP docs describe implemented rename support.

#[test]
fn lsp_docs_describe_rename_support() {
    let docs = include_str!("../../docs/lsp.md");

    for expected in [
        "### Rename",
        "Rename requests use `textDocument/prepareRename` first",
        "Supported targets are top-level functions, top-level structs",
        "top-level `let`, `const`, or `static let` bindings",
        "`textDocument/rename` validates the new identifier",
        "rejects names that",
        "would shadow an existing visible top-level binding",
        "workspace edit for matching references in open documents and workspace",
        "`.rz` files",
    ] {
        assert!(
            docs.contains(expected),
            "LSP docs should describe rename support; missing {expected:?}"
        );
    }
}
