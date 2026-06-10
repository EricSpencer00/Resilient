//! RES-3343: MCP server docs should describe integration, not scaffolding.

#[test]
fn mcp_server_docs_use_integration_wording() {
    let server = include_str!("../src/mcp_server.rs");
    let lib = include_str!("../src/lib.rs");

    for expected in [
        "## Prompts exposed (RES-2645 MCP Integration)",
        "## Resources exposed (RES-2645 MCP Integration)",
        "RES-2645: MCP integration — prompts and resources support.",
        "Prompts (RES-2645: MCP integration)",
        "Resources (RES-2645: MCP integration)",
    ] {
        assert!(
            server.contains(expected),
            "MCP server docs/comments should use integration wording: {expected:?}"
        );
    }

    assert!(
        lib.contains("MCP external-tool bridge registry — integration support"),
        "mcp_tool_registry module header should describe integration support"
    );

    for stale in ["MCP Scaffolding", "generic scaffolding for"] {
        assert!(
            !server.contains(stale) && !lib.contains(stale),
            "MCP docs/comments should not retain scaffolding wording: {stale:?}"
        );
    }
}
