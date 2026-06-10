//! RES-3249: README self-hosting status reflects the current parity harness.

#[test]
fn readme_self_hosting_section_mentions_current_parity_harness() {
    let readme = include_str!("../../README.md");

    for expected in [
        "`self-host/lexer.rz`",
        "`self-host/parser.rz`",
        "cargo test --manifest-path resilient/Cargo.toml --test self_host_parity",
        "rz self-host-parity-report --json-out artifacts/self-host-parity.json",
        "`self-host/lex.rs`",
        "`self-host/README.md`",
    ] {
        assert!(
            readme.contains(expected),
            "README self-hosting section missing {expected:?}"
        );
    }
    assert!(
        !readme.contains("Not in CI"),
        "README should not describe the self-hosting harness as outside CI"
    );
}
