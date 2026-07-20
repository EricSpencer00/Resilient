# MCP Client Examples

Copy-paste examples for talking to the Resilient MCP server: curl against
the HTTP wrapper, a small Python client, and Claude Code configuration
snippets for both the stdio and HTTP transports.

See [MCP.md](MCP.md) for protocol/tool documentation and
[MCP_DEPLOYMENT.md](MCP_DEPLOYMENT.md) for hosting guidance. The request
and response shapes below match the machine-readable contract in
[openapi.json](openapi.json).

---

## 1. curl (HTTP wrapper)

Start the server once, then issue calls against it:

```sh
rz mcp --http-port 8080
```

### Health check

```sh
curl -s http://127.0.0.1:8080/health
```

```json
{"status":"ok","service":"resilient-mcp","transport":"http","version":"0.2.0"}
```

### Calling every exposed tool

Each call POSTs to `/mcp/call` with `{"tool": "<name>", "input": {...}}`.
Tool names may use either the native `resilient_*` name or the hosted
`rz_*` alias where one exists (`rz_compile`, `rz_format`, `rz_verify`,
`rz_parse`, `rz_typecheck`, `rz_run`, `rz_lint`, `rz_check`).

```sh
# resilient_parse — syntax check
curl -s http://127.0.0.1:8080/mcp/call \
  -H 'content-type: application/json' \
  -d '{"tool":"resilient_parse","input":{"source":"fn add(int a, int b) -> int { a + b }"}}'

# resilient_typecheck — type diagnostics
curl -s http://127.0.0.1:8080/mcp/call \
  -H 'content-type: application/json' \
  -d '{"tool":"resilient_typecheck","input":{"source":"fn f() -> int { \"hello\" }"}}'

# resilient_run (alias: rz_run) — execute and capture stdout
curl -s http://127.0.0.1:8080/mcp/call \
  -H 'content-type: application/json' \
  -d '{"tool":"rz_run","input":{"source":"println(42)"}}'

# resilient_lint (alias: rz_lint) — lint warnings
curl -s http://127.0.0.1:8080/mcp/call \
  -H 'content-type: application/json' \
  -d '{"tool":"rz_lint","input":{"source":"fn F(int x) -> int { x }"}}'

# resilient_format (alias: rz_format) — pretty-print
curl -s http://127.0.0.1:8080/mcp/call \
  -H 'content-type: application/json' \
  -d '{"tool":"rz_format","input":{"source":"fn f(int x)->int{x+1}"}}'

# resilient_check (alias: rz_check) — parse + typecheck + lint
curl -s http://127.0.0.1:8080/mcp/call \
  -H 'content-type: application/json' \
  -d '{"tool":"rz_check","input":{"source":"fn add(int a, int b) -> int { a + b }"}}'

# resilient_verify (alias: rz_verify) — Z3 contract verification (requires --features z3)
curl -s http://127.0.0.1:8080/mcp/call \
  -H 'content-type: application/json' \
  -d '{"tool":"rz_verify","input":{"source":"fn div(int x, int y) -> int\n  requires y != 0\n{ x / y }"}}'

# resilient_explain_lint — human-readable lint explanation
curl -s http://127.0.0.1:8080/mcp/call \
  -H 'content-type: application/json' \
  -d '{"tool":"resilient_explain_lint","input":{"code":"L0010"}}'

# resilient_symbols — extract named symbols
curl -s http://127.0.0.1:8080/mcp/call \
  -H 'content-type: application/json' \
  -d '{"tool":"resilient_symbols","input":{"source":"fn add(int a, int b) -> int { a + b }"}}'

# resilient_hover — type info at a byte offset
curl -s http://127.0.0.1:8080/mcp/call \
  -H 'content-type: application/json' \
  -d '{"tool":"resilient_hover","input":{"source":"fn add(int a, int b) -> int { a + b }","offset":3}}'

# resilient_compile (alias: rz_compile) — bytecode summary
curl -s http://127.0.0.1:8080/mcp/call \
  -H 'content-type: application/json' \
  -d '{"tool":"rz_compile","input":{"source":"println(42)"}}'

# resilient_disasm — full bytecode disassembly
curl -s http://127.0.0.1:8080/mcp/call \
  -H 'content-type: application/json' \
  -d '{"tool":"resilient_disasm","input":{"source":"println(42)"}}'

# resilient_vm_run — execute via the register-based bytecode VM
curl -s http://127.0.0.1:8080/mcp/call \
  -H 'content-type: application/json' \
  -d '{"tool":"resilient_vm_run","input":{"source":"println(42)"}}'

# resilient_tla_check — TLC model checking on an inline spec (needs Java + tla2tools.jar)
curl -s http://127.0.0.1:8080/mcp/call \
  -H 'content-type: application/json' \
  -d '{"tool":"resilient_tla_check","input":{"spec":"---- MODULE M ----\nEXTENDS Naturals\nVARIABLE x\nInit == x = 0\nNext == x'"'"' = x + 1\n===="}}'

# resilient_fingerprint — behavioral fingerprints per function
curl -s http://127.0.0.1:8080/mcp/call \
  -H 'content-type: application/json' \
  -d '{"tool":"resilient_fingerprint","input":{"source":"fn add(int a, int b) -> int { a + b }"}}'

# resilient_resilience_score — per-function A-F resilience grade
curl -s http://127.0.0.1:8080/mcp/call \
  -H 'content-type: application/json' \
  -d '{"tool":"resilient_resilience_score","input":{"source":"fn add(int a, int b) -> int { a + b }"}}'

# resilient_contract_infer — suggest requires/ensures clauses
curl -s http://127.0.0.1:8080/mcp/call \
  -H 'content-type: application/json' \
  -d '{"tool":"resilient_contract_infer","input":{"source":"fn div(int x, int y) -> int { x / y }"}}'

# resilient_call_graph — function call graph
curl -s http://127.0.0.1:8080/mcp/call \
  -H 'content-type: application/json' \
  -d '{"tool":"resilient_call_graph","input":{"source":"fn a() -> int { b() } fn b() -> int { 1 }"}}'
```

### Response shape

Every successful call returns:

```json
{
  "status": "ok",
  "tool": "rz_run",
  "mcp_tool": "resilient_run",
  "stdout": "Output:\n42\n",
  "stderr": "",
  "diagnostics": [],
  "raw_mcp": {}
}
```

Errors (malformed body, missing tool, oversized payload, rate limit,
timeout) return `"status":"error"` with an `"error"` field and the HTTP
status codes documented in [openapi.json](openapi.json) (`400`, `404`,
`413`, `429`, `504`).

---

## 2. Python client

Uses only the standard library (`urllib.request`) — no extra
dependencies required.

```python
#!/usr/bin/env python3
"""Minimal Python client for the Resilient MCP HTTP wrapper.

Usage:
    python3 mcp_client.py rz_run '{"source": "println(42)"}'
"""
import json
import sys
import urllib.request

# Bypass system proxy auto-detection, which can otherwise intercept
# plain http://127.0.0.1 requests even with no proxy configured.
_opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))


def call_tool(base_url: str, tool: str, tool_input: dict) -> dict:
    payload = json.dumps({"tool": tool, "input": tool_input}).encode("utf-8")
    request = urllib.request.Request(
        f"{base_url}/mcp/call",
        data=payload,
        headers={"content-type": "application/json"},
        method="POST",
    )
    try:
        with _opener.open(request, timeout=15) as response:
            return json.loads(response.read().decode("utf-8"))
    except urllib.error.HTTPError as exc:
        return json.loads(exc.read().decode("utf-8"))


def health(base_url: str) -> dict:
    with _opener.open(f"{base_url}/health", timeout=5) as response:
        return json.loads(response.read().decode("utf-8"))


if __name__ == "__main__":
    base_url = "http://127.0.0.1:8080"
    tool = sys.argv[1] if len(sys.argv) > 1 else "rz_run"
    tool_input = json.loads(sys.argv[2]) if len(sys.argv) > 2 else {"source": "println(42)"}

    print("health:", health(base_url))
    result = call_tool(base_url, tool, tool_input)
    print(json.dumps(result, indent=2))
```

Run it against a locally running server:

```sh
rz mcp --http-port 8080 &
python3 docs/mcp_client.py resilient_run '{"source": "println(42)"}'
```

A runnable copy of this script lives at
[`examples/mcp_client.py`](../examples/mcp_client.py).

---

## 3. Claude Code MCP configuration

### stdio transport (default, local process)

Claude Code spawns `rz mcp` as a subprocess and talks NDJSON over
stdin/stdout — no network port needed:

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

### HTTP transport (hosted / remote server)

If the server is already running (locally via `rz mcp --http-port 8080`
or on a hosted deployment per [MCP_DEPLOYMENT.md](MCP_DEPLOYMENT.md)),
point Claude Code at the HTTP endpoint instead of spawning a process:

```json
{
  "mcpServers": {
    "resilient-http": {
      "url": "http://127.0.0.1:8080/mcp/call",
      "transport": "http"
    }
  }
}
```

For a publicly hosted instance, replace the URL with the deployed
endpoint (e.g. `https://resilient.example.com/mcp/call`) and put auth
in front of it per the production checklist in
[MCP_DEPLOYMENT.md](MCP_DEPLOYMENT.md#production-checklist) — the
wrapper itself does not authenticate requests.

---

## Tool reference

The full list of 17 native tool names, current as of schema v1 (see
[MCP_SCHEMA_CHANGELOG.md](MCP_SCHEMA_CHANGELOG.md)):

`resilient_parse`, `resilient_typecheck`, `resilient_run`,
`resilient_lint`, `resilient_format`, `resilient_check`,
`resilient_verify`, `resilient_explain_lint`, `resilient_symbols`,
`resilient_hover`, `resilient_compile`, `resilient_disasm`,
`resilient_vm_run`, `resilient_tla_check`, `resilient_fingerprint`,
`resilient_resilience_score`, `resilient_contract_infer`,
`resilient_call_graph`.

Hosted aliases (`rz_*`) exist for the eight most common tools:
`rz_compile`, `rz_format`, `rz_verify`, `rz_parse`, `rz_typecheck`,
`rz_run`, `rz_lint`, `rz_check`.
