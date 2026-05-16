# Resilient MCP Server

The Resilient compiler ships a built-in [Model Context Protocol](https://modelcontextprotocol.io)
(MCP) server that exposes the full compilation pipeline as tools. AI assistants
that speak MCP (Claude Desktop, Cursor, VS Code with the MCP extension, …) can
use these tools to write, check, and run Resilient code without leaving their
chat session.

---

## Quick start

```sh
# Activate the MCP server
rz mcp
```

The server reads one JSON-RPC message per line from stdin and writes one
response per line to stdout (NDJSON — newline-delimited JSON, per the MCP
2024-11-05 spec). You typically don't run it by hand; instead, register it
in your MCP client's config:

```json
{
  "mcpServers": {
    "resilient": {
      "command": "rz",
      "args": ["mcp"]
    }
  }
}
```

---

## Tools

### `resilient_parse`

Parse Resilient source and report syntax errors.

**Input:**
```json
{ "source": "fn add(int a, int b) -> int { a + b }" }
```

**Success output:**
```
OK — parsed 1 top-level statement(s), no errors.
```

**Error output:**
```
Parse errors (1):
1:4: expected identifier
```

---

### `resilient_typecheck`

Parse + type-check source. Returns all type diagnostics.

**Input:**
```json
{ "source": "fn f() -> int { \"hello\" }" }
```

**Error output:**
```
Type error:
1:17: return type mismatch: expected int, got string
```

---

### `resilient_run`

Execute Resilient source and capture stdout.

**Input:**
```json
{ "source": "println(\"hello, world!\")" }
```

**Success output:**
```
Output:
hello, world!
```

---

### `resilient_lint`

Run all Resilient lint passes and return warnings.

Includes: naming conventions, dead code, unsafe call patterns,
safety-critical violations, AI-threat detection, and more.

**Input:**
```json
{ "source": "fn F(int x) -> int { x }" }
```

---

### `resilient_format`

Format / pretty-print Resilient source using the canonical formatter.

**Input:**
```json
{ "source": "fn f(int x)->int{x+1}" }
```

**Output:**
```
fn f(int x) -> int {
  x + 1
}
```

---

### `resilient_check`

Full pipeline: parse + typecheck + all lint passes. Fastest way to
validate a snippet end-to-end.

**Input:**
```json
{ "source": "fn add(int a, int b) -> int { a + b }" }
```

**Success output:**
```
OK — parse, typecheck, and lint all passed.
```

---

### `resilient_verify`

Z3 SMT contract verification. Checks `requires` / `ensures` clauses on
every function.

> **Note:** Only available in builds compiled with `--features z3`.
> Without Z3, the tool returns a clear "not available" message.

**Input:**
```json
{
  "source": "fn div(int x, int y) -> int\n  requires y != 0\n{ x / y }"
}
```

---

## Protocol notes

- **Transport:** stdio (NDJSON — one JSON object per line, flush after each)
- **Protocol version:** `2024-11-05`
- **No feature flags required:** the MCP server is always available on
  native builds (it only depends on `serde_json`, which is already an
  unconditional dependency of the compiler).
- **wasm32:** not available (same constraint as the REPL and watch mode).

---

## Registry

The Resilient MCP server is published to the [official MCP Registry](https://registry.modelcontextprotocol.io)
under the namespace `io.github.ericspencer00/resilient`. The registration
points at the multi-arch Docker image at `ghcr.io/ericspencer00/resilient`
(both `amd64` and `arm64`), so any MCP client that resolves servers from
the registry can install Resilient with a single click without needing
to clone or build from source.

`server.json` at the repo root is the source of truth for the registry
entry; `.github/workflows/mcp-publish.yml` re-publishes it on every
`release: published` event, syncing the version + OCI tag to the release.

## Example session

```
→ {"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}
← {"jsonrpc":"2.0","id":1,"result":{"capabilities":{"tools":{}},"protocolVersion":"2024-11-05","serverInfo":{"name":"resilient","version":"0.2.0"}}}

→ {"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}
← {"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"resilient_parse",...},...]}}

→ {"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"resilient_run","arguments":{"source":"println(42)"}}}
← {"jsonrpc":"2.0","id":3,"result":{"content":[{"text":"Output:\n42\n","type":"text"}],"isError":false}}
```
