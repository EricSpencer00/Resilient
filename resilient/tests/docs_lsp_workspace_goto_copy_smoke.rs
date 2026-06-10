//! RES-3261: LSP docs describe workspace go-to-definition support.

#[test]
fn lsp_docs_describe_workspace_go_to_definition_support() {
    let docs = include_str!("../../docs/lsp.md");

    for expected in [
        "jumps to its declaration site in the current file or an imported",
        "Workspace lookup follows `use \"...\"` imports",
        "including unopened files under the",
        "initialized workspace folder",
    ] {
        assert!(
            docs.contains(expected),
            "LSP docs should describe workspace go-to-definition support; missing {expected:?}"
        );
    }

    assert!(
        !docs.contains("multi-file workspace lookup is a follow-up"),
        "LSP docs should not describe workspace go-to-definition as future-only"
    );
    assert!(
        !docs.contains("- Multi-file workspace go-to-definition."),
        "LSP docs should not list implemented workspace go-to-definition as next work"
    );
}
