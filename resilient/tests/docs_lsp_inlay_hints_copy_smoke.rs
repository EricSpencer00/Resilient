//! RES-3265: LSP docs describe implemented inlay hint support.

#[test]
fn lsp_docs_describe_inlay_hint_support() {
    let docs = include_str!("../../docs/lsp.md");

    for expected in [
        "## Inlay hints",
        "The server advertises `textDocument/inlayHint` support.",
        "enabled by default for inferred `let` bindings",
        "omitted function",
        "return types",
        "`resilient.inlayHints.types: false`",
        "Parameter hints for user-function call sites are opt-in.",
        "`resilient.inlayHints.parameters: true`",
    ] {
        assert!(
            docs.contains(expected),
            "LSP docs should describe inlay hint support; missing {expected:?}"
        );
    }
}
