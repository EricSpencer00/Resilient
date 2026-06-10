//! RES-3349: free-variable docs should describe current consumers.

#[test]
fn free_vars_docs_describe_current_consumers() {
    let lib = include_str!("../src/lib.rs");
    let free_vars = include_str!("../src/free_vars.rs");

    for expected in [
        "RES-164a: reusable pure free-variable analysis on the AST.",
        "Used by recovery checking today and kept runtime-free",
        "free_vars` is consumed by recovery\n//! checking",
        "this module's regression tests today",
        "reuse it for JIT closure capture without\n//! threading runtime `Environment` state",
    ] {
        assert!(
            lib.contains(expected) || free_vars.contains(expected),
            "free-variable docs should describe current consumers: {expected:?}"
        );
    }

    for stale in [
        "Phase-K\n// scaffolding for JIT closure capture",
        "only consumed from the\n//! tests in this module today",
        "RES-164c/d will wire it into the\n//! JIT lowering",
    ] {
        assert!(
            !lib.contains(stale) && !free_vars.contains(stale),
            "free-variable docs should not retain stale scaffolding wording: {stale:?}"
        );
    }
}
