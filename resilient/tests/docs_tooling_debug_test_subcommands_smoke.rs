//! RES-3277: tooling docs describe current debug and test subcommands.

#[test]
fn tooling_docs_describe_current_debug_and_test_subcommands() {
    let docs = include_str!("../../docs/tooling.md");

    for expected in [
        "## Debugger",
        "### `rz debug <file>`",
        "Starts the Debug Adapter Protocol (DAP) server on stdin/stdout",
        "rz debug examples/hello.rz",
        "For direct adapter launches, clients may also use `rz --dap`.",
        "## Test framework",
        "### `rz test [<file|dir>] [--filter <substring>]`",
        "Discovers and runs `fn test_*()` functions in `.rz` files.",
        "rz test resilient/examples/test_runner_demo.rz",
        "Parallel execution and JUnit output are still future",
    ] {
        assert!(
            docs.contains(expected),
            "tooling docs should describe current debug/test tooling; missing {expected:?}"
        );
    }

    for stale in [
        "There is no standalone step-debugger today.",
        "DAP\nserver) is tracked as a future deliverable",
        "there is no `rz test`\nsubcommand yet",
        "A first-class `rz test` runner",
    ] {
        assert!(
            !docs.contains(stale),
            "tooling docs should not retain stale future-only copy: {stale:?}"
        );
    }
}
