//! RES-3353: JIT run-path comments should describe current behavior.

#[test]
fn jit_run_path_comment_uses_current_backend_wording() {
    let source = include_str!("../src/lib.rs");

    for expected in [
        "RES-072 / RES-096: Cranelift JIT path for the supported",
        "tree-walker subset. Unsupported AST shapes surface cleanly",
        "callers can fall back without a panic or opaque message",
    ] {
        assert!(
            source.contains(expected),
            "JIT run path comment should describe current behavior: {expected:?}"
        );
    }

    for stale in [
        "RES-072 Phase A: Cranelift JIT path",
        "Stub today; RES-096+",
        "will add real AST lowering",
        "the user knows the JIT isn't implemented yet",
    ] {
        assert!(
            !source.contains(stale),
            "JIT run path comment should not retain stale stub wording: {stale:?}"
        );
    }
}
