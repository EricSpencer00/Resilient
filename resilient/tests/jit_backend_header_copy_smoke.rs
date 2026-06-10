//! RES-3339: JIT backend header describes the current implementation.

#[test]
fn jit_backend_header_uses_current_backend_wording() {
    let source = include_str!("../src/jit_backend.rs");

    for expected in [
        "RES-072 / RES-096: Cranelift JIT backend.",
        "lowers the currently supported tree-walker subset to\n//! native code",
        "Unsupported AST shapes return `JitError::Unsupported(...)` cleanly",
        "fall back to the interpreter instead of panicking",
    ] {
        assert!(
            source.contains(expected),
            "JIT backend header should describe current behavior: {expected:?}"
        );
    }

    for stale in [
        "stub\n//! `run`",
        "Phase B (this\n//! revision)",
        "Future\n//! tickets layer on",
    ] {
        assert!(
            !source.contains(stale),
            "JIT backend header should not retain stale revision wording: {stale:?}"
        );
    }
}
