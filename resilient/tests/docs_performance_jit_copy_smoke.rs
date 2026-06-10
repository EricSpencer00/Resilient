//! RES-3369: performance docs state current JIT limitations as support boundaries.

#[test]
fn performance_docs_describe_jit_limitations_without_planned_labels() {
    let docs = include_str!("../../docs/performance.md");

    for expected in [
        "What it doesn't yet do (use the VM or tree walker for these today):",
        "Reassignment (`x = x + 1`) — tracked by RES-107",
        "`while` loops — tracked by RES-107",
    ] {
        assert!(
            docs.contains(expected),
            "performance docs should state current JIT support boundaries: {expected:?}"
        );
    }

    for stale in [
        "What it doesn't yet do (use the VM instead):",
        "RES-107 (planned)",
    ] {
        assert!(
            !docs.contains(stale),
            "performance docs should not use stale planned-limit wording: {stale:?}"
        );
    }
}
