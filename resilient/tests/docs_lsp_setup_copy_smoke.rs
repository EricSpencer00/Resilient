//! RES-3219: LSP docs use the current `rz` binary and resilient filetype.

#[test]
fn lsp_docs_use_rz_and_resilient_filetype() {
    let doc = include_str!("../../docs/lsp.md");

    for expected in [
        r#"cmd      = { "/absolute/path/to/rz", "--lsp" }"#,
        r#"filetypes = { "resilient" }"#,
        "map\n`.rz` files to it",
        "set the language\nID to `resilient` for `.rz` files",
    ] {
        assert!(
            doc.contains(expected),
            "LSP docs missing current setup copy {expected:?}"
        );
    }
    for retired in [
        "/absolute/path/to/resilient",
        r#"filetypes = { "rust" }"#,
        "ID to `rust`",
    ] {
        assert!(
            !doc.contains(retired),
            "LSP docs should not use retired setup copy {retired:?}"
        );
    }
}
