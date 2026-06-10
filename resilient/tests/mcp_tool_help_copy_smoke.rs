//! RES-3341: MCP tool help should describe integration, not scaffolding.

#[test]
fn mcp_tool_registry_uses_integration_wording() {
    let source = include_str!("../src/mcp_tool_registry.rs");

    for expected in [
        "generic integration framework",
        "rz tool — external tool bridge (MCP integration)",
    ] {
        assert!(
            source.contains(expected),
            "MCP tool registry copy should use integration wording: {expected:?}"
        );
    }

    for stale in ["generic scaffolding framework", "MCP scaffolding"] {
        assert!(
            !source.contains(stale),
            "MCP tool registry copy should not retain scaffolding wording: {stale:?}"
        );
    }
}
