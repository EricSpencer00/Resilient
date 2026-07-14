//! RES-3353 / RES-4019: JIT run-path comments should describe current
//! behavior.
//!
//! Test changes (RES-4019, track B-E4): the previous wording pinned
//! here ("Unsupported AST shapes surface cleanly ... callers can fall
//! back without a panic or opaque message") described the *old*
//! contract — `--jit` returned a clean `JitError` but the CLI still
//! surfaced it as a hard error; nothing actually fell back. RES-4019
//! makes that literal: `--jit`'s dispatch site now transparently
//! retries on the VM for every `JitError::is_precompile()` case. The
//! wording pinned below was updated to match, per this repo's test
//! discipline (a stale assertion isn't grounds to weaken the check —
//! it's grounds to fix it to track the new, more accurate comment).

#[test]
fn jit_run_path_comment_uses_current_backend_wording() {
    let source = include_str!("../../src/lib.rs");

    for expected in [
        "RES-072 / RES-096: Cranelift JIT path for the supported",
        "RES-4019 (B-E4): a JIT error that's detectable before any",
        "transparently falls back to",
        "the VM instead of surfacing a hard error",
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
