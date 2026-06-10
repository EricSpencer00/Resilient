//! RES-3305: stable inventory splits documented debugger status from profiler future work.

#[test]
fn stable_inventory_splits_debugger_from_future_profiler() {
    let inventory = include_str!("../../docs/stable-regression-inventory.md");
    let tooling = include_str!("../../docs/tooling.md");

    for expected in [
        "## Debugger",
        "### `rz debug <file>`",
        "For direct adapter launches, clients may also use `rz --dap`.",
        "## Profiler",
        "There is no profiler today.",
    ] {
        assert!(
            tooling.contains(expected),
            "tooling docs should keep debugger documented and profiler future status explicit; missing {expected:?}"
        );
    }

    for expected in [
        "| `rz debug <file>` / `--dap` | `resilient/tests/debug_help_smoke.rs`, `resilient/src/dap_server.rs` | Covered | DAP server CLI entrypoints are documented and help-covered; breakpoints, stepping, and watch expressions remain maturing. |",
        "| Profiler path | `docs/tooling.md` documents the profiler as future; current timing data comes from `rz bench` and `--jit-cache-stats`. | Stabilize a profiler CLI and add direct smoke coverage before promoting it. |",
    ] {
        assert!(
            inventory.contains(expected),
            "stable inventory should split debugger and profiler status; missing {expected:?}"
        );
    }

    for stale in [
        "| Debugger / profiler paths |",
        "Public docs classify them as future work.",
        "Stabilize the user-facing workflows before adding them to this inventory.",
    ] {
        assert!(
            !inventory.contains(stale),
            "stable inventory should not retain combined debugger/profiler future wording: {stale:?}"
        );
    }
}
