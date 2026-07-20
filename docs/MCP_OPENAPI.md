# MCP HTTP wrapper — OpenAPI contract

`docs/openapi.json` is a hand-written OpenAPI 3.0 document describing the
HTTP surface exposed by `rz mcp --http-port <addr>`: `GET /health` and
`POST /mcp/call`, their request/response schemas, and every status code the
wrapper can return (`200`, `400`, `404`, `413`, `429`, `504`).

It exists so external tooling (client SDK generators, contract tests,
API gateways) has a machine-readable contract instead of having to read
`resilient/src/mcp_server.rs` or the prose in [`MCP.md`](MCP.md).

`resilient/tests/mcp_openapi_contract_smoke.rs` spawns the real `rz`
binary and asserts that live server responses conform to the schemas in
this document — the doc and the implementation are checked to stay in
sync in CI.

See [`MCP.md`](MCP.md) for the full prose walkthrough of the MCP server,
its tools, and its hardening/concurrency/logging behavior.

RES-3961. Part of the Live MCP Server initiative (#3934).
