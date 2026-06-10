//! RES-3283: STDLIB contributor guidance follows the library split.

#[test]
fn stdlib_docs_point_builtin_contributors_at_library_sources() {
    let docs = include_str!("../../STDLIB.md");

    for expected in [
        "Implementations live in `resilient/src/lib.rs` (table: `BUILTINS`);",
        "signatures live in `resilient/src/typechecker.rs`",
        "1. The `BUILTINS` table in `resilient/src/lib.rs`.",
        "5. A focused Rust test in `resilient/src/lib.rs` or `resilient/tests/`.",
    ] {
        assert!(
            docs.contains(expected),
            "STDLIB.md should use current builtin contributor locations; missing {expected:?}"
        );
    }

    assert!(
        !docs.contains("resilient/src/main.rs"),
        "STDLIB.md should not send builtin contributors to the CLI shim"
    );
}
