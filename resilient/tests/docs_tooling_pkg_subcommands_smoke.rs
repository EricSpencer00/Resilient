//! RES-3275: tooling docs describe current `rz pkg` subcommands.

#[test]
fn tooling_docs_describe_current_pkg_subcommands() {
    let docs = include_str!("../../docs/tooling.md");

    for expected in [
        "## Package tooling",
        "### `rz pkg init <name>`",
        "### `rz pkg add <name> <spec>`",
        "rz pkg add mylib path:../libs/mylib",
        "rz pkg add netutil git:https://github.com/user/netutil --rev abc123",
        "### `rz pkg publish --dry-run`",
        "real registry POST path is still future",
        "`--dry-run` is required today",
    ] {
        assert!(
            docs.contains(expected),
            "tooling docs should describe current package tooling; missing {expected:?}"
        );
    }

    for stale in [
        "`rz pkg` is the umbrella for future package operations",
        "only `init` exists today",
    ] {
        assert!(
            !docs.contains(stale),
            "tooling docs should not retain stale package-tooling copy: {stale:?}"
        );
    }
}
