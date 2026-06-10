//! RES-3293: playground docs describe the current tree-walker integration.

#[test]
fn playground_docs_describe_current_tree_walker_integration() {
    let readme = include_str!("../../playground/README.md");
    let manifest = include_str!("../../playground/Cargo.toml");
    let wasm_entry = include_str!("../../playground/src/lib.rs");

    for expected in [
        "`compile_and_run` now calls the real",
        "`resilient::run_program` tree-walker",
        "`flavor: \"tree-walker\"`",
        "The playground is a browser demo surface, not the full native CLI.",
        "JIT, FFI, Z3-backed\nverification, file I/O, the REPL, and watch mode",
        "cfg-gates native-only dependencies",
        "reserved for future stdin-style examples",
    ] {
        assert!(
            readme.contains(expected),
            "playground README should describe current tree-walker status; missing {expected:?}"
        );
    }

    assert!(
        manifest.contains("description = \"RES-368: WASM-targeted Resilient web playground.\""),
        "playground manifest should not describe the crate as a scaffold"
    );
    assert!(
        wasm_entry.contains("resilient::run_program(source)")
            && wasm_entry.contains("flavor: \"tree-walker\""),
        "playground WASM entry should still call the real tree-walker"
    );

    for stale in [
        "This is the **scaffold** PR",
        "interpreter integration is **stubbed**",
        "echoes the source with a \"scaffold\" notice",
        "The `resilient` crate is currently `[[bin]]`-only",
        "editing `resilient/src/main.rs`",
        "res-333-supervisor-fresh",
        "The follow-up ticket for the lib refactor",
        "WASM-targeted scaffold",
    ] {
        assert!(
            !readme.contains(stale) && !manifest.contains(stale),
            "playground docs should not retain stale scaffold/lib-split language: {stale:?}"
        );
    }
}
