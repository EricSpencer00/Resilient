//! RES-3357: playground runtime comments should describe current behavior.

#[test]
fn playground_runtime_comments_use_tree_walker_wording() {
    let wasm_entry = include_str!("../../playground/src/lib.rs");
    let browser_js = include_str!("../../playground/web/main.js");

    for expected in [
        "RES-510 PR 3 calls into the real `resilient::run_program`",
        "tree-walker. The lib refactor",
        "before the WASM tree-walker call",
        "Fast snippets should still show\n  // the pending state consistently",
    ] {
        assert!(
            wasm_entry.contains(expected) || browser_js.contains(expected),
            "playground runtime comments should describe current tree-walker behavior: {expected:?}"
        );
    }

    for stale in [
        "previously a stub",
        "Stubs are fast",
        "full interpreter will benefit from this once integrated",
    ] {
        assert!(
            !wasm_entry.contains(stale) && !browser_js.contains(stale),
            "playground runtime comments should not retain stub-era wording: {stale:?}"
        );
    }
}
