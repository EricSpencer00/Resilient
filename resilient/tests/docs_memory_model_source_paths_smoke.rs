//! RES-3287: memory-model docs follow the compiler library split.

#[test]
fn memory_model_docs_use_current_host_interpreter_source_paths() {
    let docs = include_str!("../../docs/memory-model.md");

    for expected in [
        "| Host interpreter         | `resilient/src/lib.rs`",
        "Defined by the `Value` enum in `resilient/src/lib.rs`.",
        "The host implementation is `eval_live_block` in\n`resilient/src/lib.rs`.",
    ] {
        assert!(
            docs.contains(expected),
            "memory model docs should point host implementation notes at lib.rs; missing {expected:?}"
        );
    }

    assert!(
        !docs.contains("resilient/src/main.rs"),
        "memory model docs should not point implementation notes at the CLI shim"
    );
}
