//! RES-3273: LSP docs describe current diagnostic range behavior.

#[test]
fn lsp_docs_describe_current_diagnostic_ranges() {
    let docs = include_str!("../../docs/lsp.md");

    for expected in [
        "Every `did_open` or `did_change` event re-runs the full parse +",
        "reported column for parser and typechecker errors",
        "lint diagnostics use",
        "the source positions recorded by the lint pass",
    ] {
        assert!(
            docs.contains(expected),
            "LSP docs should describe current diagnostic ranges; missing {expected:?}"
        );
    }

    assert!(
        !docs.contains("Parser errors currently appear at"),
        "LSP docs should not say parser errors use the old fallback range"
    );
    assert!(
        !docs.contains("line 1, column 1"),
        "LSP docs should not describe parser diagnostics as fixed at 1:1"
    );
}
