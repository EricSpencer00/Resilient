//! RES-3291: live-block docs avoid stale main.rs line anchors.

#[test]
fn live_block_docs_use_symbol_oriented_library_references() {
    let docs = include_str!("../../docs/live-block-semantics.md");

    for expected in [
        "Implementation references below name symbols in `resilient/src/lib.rs`",
        "`env_snapshot` inside `eval_live_block`",
        "`LiveRetryGuard`",
        "retry loop in `eval_live_block`",
        "`DEFAULT_LIVE_MAX_RETRIES = 3`",
        "AST node as `invariants: Vec<Node>`",
        "default `BackoffConfig` policy",
        "anchored at block entry inside `eval_live_block`",
        "`self.statics`, which is shared across attempts by design",
        "registered in the `BUILTINS` table",
    ] {
        assert!(
            docs.contains(expected),
            "live-block docs should use current symbol-oriented references; missing {expected:?}"
        );
    }

    for stale in [
        "resilient/src/main.rs",
        "main.rs` line",
        "`main.rs` line",
        "line ~",
    ] {
        assert!(
            !docs.contains(stale),
            "live-block docs should not retain stale main.rs line anchors: {stale:?}"
        );
    }
}
