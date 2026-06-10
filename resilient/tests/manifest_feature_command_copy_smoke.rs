//! RES-3231: manifest feature comments use the current `rz` command names.

#[test]
fn manifest_feature_comments_use_rz_commands() {
    let manifest = include_str!("../Cargo.toml");

    for expected in [
        "run `rz --lsp`",
        "run `rz --jit prog.rz`",
        "compiled `rz` binary",
    ] {
        assert!(
            manifest.contains(expected),
            "manifest missing current command copy {expected:?}"
        );
    }
    for retired in [
        "run `resilient --lsp`",
        "run `resilient --jit prog.rs`",
        "compiled `resilient` binary",
    ] {
        assert!(
            !manifest.contains(retired),
            "manifest should not use retired command copy {retired:?}"
        );
    }
}
