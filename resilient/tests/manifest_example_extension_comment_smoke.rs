//! RES-3229: manifest comments describe the current `.rz` example corpus.

#[test]
fn manifest_example_comment_uses_current_extension() {
    let manifest = include_str!("../Cargo.toml");

    assert!(
        manifest.contains("`examples/*.rz` holds Resilient-language source files"),
        "manifest should describe the current .rz example corpus"
    );
    assert!(
        manifest.contains("autoexamples = false"),
        "manifest should still disable Cargo example auto-discovery"
    );
    assert!(
        !manifest.contains("`examples/*.rs` holds Resilient-language source files"),
        "manifest should not mention the retired .rs example corpus"
    );
}
