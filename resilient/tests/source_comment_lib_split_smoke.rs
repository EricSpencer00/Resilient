//! RES-3295: source comments reflect the compiler library split.

#[test]
fn source_comments_use_current_lib_split_language() {
    let typechecker = include_str!("../src/typechecker.rs");
    let interpreter_bench = include_str!("../benches/interpreter.rs");

    for expected in [
        "Keep in sync with `resilient/src/lib.rs::BUILTINS`",
        "minus the names in `IMPURE_BUILTINS`",
    ] {
        assert!(
            typechecker.contains(expected),
            "typechecker comments should reference current builtin source; missing {expected:?}"
        );
    }

    for expected in [
        "through the compiled\n//! `rz` binary",
        "measures the CLI path users invoke",
        "including argument\n//! parsing, diagnostics, process startup, and stdout capture",
    ] {
        assert!(
            interpreter_bench.contains(expected),
            "interpreter bench comment should describe current benchmark rationale; missing {expected:?}"
        );
    }

    for stale in [
        "resilient/src/main.rs::BUILTINS",
        "resilient/src/main.rs` is a 16k-line binary crate",
        "no\n//! public `lib.rs`",
        "compiled\n//! `resilient` binary",
    ] {
        assert!(
            !typechecker.contains(stale) && !interpreter_bench.contains(stale),
            "source comments should not retain stale lib-split wording: {stale:?}"
        );
    }
}
