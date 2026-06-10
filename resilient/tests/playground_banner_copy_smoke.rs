//! RES-3351: playground fallback banner uses user-facing copy.

#[test]
fn playground_banner_uses_neutral_fallback_copy() {
    let html = include_str!("../../playground/web/index.html");
    let js = include_str!("../../playground/web/main.js");
    let wasm_entry = include_str!("../../playground/src/lib.rs");

    for expected in [
        r#"id="fallback-banner""#,
        "Limited playground build &mdash; execution is unavailable",
        r#"getElementById("fallback-banner")"#,
        "fallback banner remains\n    /// hidden for any non-`\"stub\"` flavor",
    ] {
        assert!(
            html.contains(expected) || js.contains(expected) || wasm_entry.contains(expected),
            "playground banner copy should use neutral fallback wording: {expected:?}"
        );
    }

    for stale in [
        "scaffold-banner",
        "scaffold build",
        "interpreter integration pending",
        "page's \"scaffold\" banner",
    ] {
        assert!(
            !html.contains(stale) && !js.contains(stale) && !wasm_entry.contains(stale),
            "playground banner should not expose internal scaffold wording: {stale:?}"
        );
    }
}
