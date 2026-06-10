//! RES-3303: playground acceptance rows match the tree-walker integration.

#[test]
fn playground_acceptance_table_no_longer_describes_stub_output() {
    let readme = include_str!("../../playground/README.md");
    let wasm_entry = include_str!("../../playground/src/lib.rs");

    for expected in [
        "`resilient-playground` builds against the compiler library target",
        "calls the real `resilient::run_program` tree-walker",
        "native CLI-only pieces stay cfg-gated",
        "round-trip returns stdout, diagnostics, exit code, duration",
        "`flavor: \"tree-walker\"`",
        "no stub output remains",
    ] {
        assert!(
            readme.contains(expected),
            "playground acceptance table should reflect tree-walker wiring; missing {expected:?}"
        );
    }

    assert!(
        wasm_entry.contains("resilient::run_program(source)")
            && wasm_entry.contains("flavor: \"tree-walker\""),
        "playground entrypoint should still call the real tree-walker"
    );

    for stale in [
        "stub interpreter",
        "full integration pending the lib refactor",
        "output is a stub message",
        "until full integration",
    ] {
        assert!(
            !readme.contains(stale),
            "playground acceptance table should not retain stub-era wording: {stale:?}"
        );
    }
}
