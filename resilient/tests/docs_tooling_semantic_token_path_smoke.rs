//! RES-3289: tooling docs point semantic-token readers at lib.rs.

#[test]
fn tooling_docs_use_current_semantic_token_source_path() {
    let tooling = include_str!("../../docs/tooling.md");
    let lsp_server = include_str!("../src/lsp_server.rs");

    assert!(
        tooling.contains("see `sem_tok` in\n  `resilient/src/lib.rs`"),
        "tooling docs should point semantic-token readers at lib.rs"
    );
    assert!(
        !tooling.contains("resilient/src/main.rs"),
        "tooling docs should not point semantic-token readers at the CLI shim"
    );

    assert!(
        lsp_server.contains("the `sem_tok::*` token-type indices declared in `lib.rs`"),
        "LSP source comment should name the current sem_tok source file"
    );
    assert!(
        !lsp_server.contains("indices declared in `main.rs`"),
        "LSP source comment should not refer to the old monolithic file"
    );
}
