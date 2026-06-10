//! RES-3359: MCP syntax resource states current support boundaries.

#[test]
fn mcp_syntax_resource_uses_not_yet_supported_wording() {
    let source = include_str!("../src/mcp_server.rs");

    for expected in [
        "Array<T>          // typed array (not yet supported)",
        "fn(x) { x + 1 }                // type-inferred params (not yet supported)",
    ] {
        assert!(
            source.contains(expected),
            "MCP syntax resource should label unsupported examples clearly: {expected:?}"
        );
    }

    for stale in [
        "Array<T>          // typed array (planned)",
        "fn(x) { x + 1 }                // type-inferred params (planned)",
    ] {
        assert!(
            !source.contains(stale),
            "MCP syntax resource should not use vague planned wording: {stale:?}"
        );
    }
}
