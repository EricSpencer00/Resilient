//! RES-3269: LSP docs describe implemented symbol support.

#[test]
fn lsp_docs_describe_symbol_support() {
    let docs = include_str!("../../docs/lsp.md");

    for expected in [
        "### Symbols",
        "`textDocument/documentSymbol` returns a source-ordered outline",
        "top-level functions, structs, and type aliases",
        "`workspace/symbol` indexes `.rz` files",
        "under the initialized workspace",
        "matching top-level symbols across those files",
    ] {
        assert!(
            docs.contains(expected),
            "LSP docs should describe symbol support; missing {expected:?}"
        );
    }
}
