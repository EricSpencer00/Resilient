//! RES-3271: LSP docs describe implemented code action quick fixes.

#[test]
fn lsp_docs_describe_code_action_support() {
    let docs = include_str!("../../docs/lsp.md");

    for expected in [
        "### Code actions",
        "`textDocument/codeAction` offers quick fixes derived from diagnostics.",
        "adding `requires true;` / `ensures true;`",
        "contract stubs for no-contract lint diagnostics",
        "inserting a missing",
        "semicolon",
        "suppressing lint diagnostics with `// resilient: allow`",
        "prefixing unused variables or dead functions with `_`",
        "adding numeric",
        "`as <type>` casts for type mismatches",
        "adding `use \"...\"` imports",
        "undefined names found in the workspace index",
    ] {
        assert!(
            docs.contains(expected),
            "LSP docs should describe code action support; missing {expected:?}"
        );
    }
}
