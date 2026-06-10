//! RES-3325: LSP source comments point integration touchpoints at lib.rs.

#[test]
fn lsp_comments_use_lib_rs_touchpoints() {
    let source = include_str!("../src/lsp_server.rs");

    for expected in [
        "`mod lsp_server;` declaration in `lib.rs`",
        "Invoked from the library CLI dispatcher when `--lsp`",
        "constants in lib.rs",
        "\"span unreliability\" note in lib.rs",
    ] {
        assert!(
            source.contains(expected),
            "LSP comments should include current lib.rs wording: {expected:?}"
        );
    }

    for stale in [
        "`mod lsp_server;` declaration in `main.rs`",
        "Invoked from `main()` when `--lsp`",
        "constants in main.rs",
        "\"span unreliability\" note in main.rs",
    ] {
        assert!(
            !source.contains(stale),
            "LSP comments should not retain stale main.rs wording: {stale:?}"
        );
    }
}
