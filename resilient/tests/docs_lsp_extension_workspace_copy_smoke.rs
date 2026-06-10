//! RES-3331: LSP docs describe the VS Code extension as a workspace.

#[test]
fn lsp_docs_use_workspace_language_for_vscode_extension() {
    let lsp_root = include_str!("../../LSP.md");
    let lsp_docs = include_str!("../../docs/lsp.md");

    for expected in [
        "bundled `vscode-extension/` workspace",
        "contains a minimal VS Code\nextension workspace",
        "`npm install && vsce package` inside it\nproduces an installable `.vsix`",
    ] {
        assert!(
            lsp_root.contains(expected) || lsp_docs.contains(expected),
            "LSP docs should use workspace/package wording: {expected:?}"
        );
    }

    for stale in ["`vscode-extension/` scaffold", "extension scaffold"] {
        assert!(
            !lsp_root.contains(stale) && !lsp_docs.contains(stale),
            "LSP docs should not describe the VS Code extension as a scaffold: {stale:?}"
        );
    }
}
