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

For hosted integrations that cannot spawn a local stdio process, run the
HTTP wrapper:

```sh
rz mcp --http-port 8080
curl http://127.0.0.1:8080/health
curl -s http://127.0.0.1:8080/mcp/call \
  -H 'content-type: application/json' \
  -d '{"tool":"rz_format","input":{"source":"fn f(int x)->int{x+1}"}}'
```

The wrapper exposes `GET /health`, `POST /mcp/call`, and `GET /metrics`.
Tool names may use the hosted aliases from RES-3782 (`rz_compile`,
`rz_format`, `rz_verify`, and related `rz_*` names) or the native MCP names
(`resilient_compile`, `resilient_format`, `resilient_verify`, ...).

### Hardening (Phase 1, RES-3934/3935/3936/3938/3944)

The HTTP wrapper enforces three limits, all configurable via environment
variables, all with sane defaults so a bare `rz mcp --http-port` stays
safe out of the box:

| Limit | Env var | Default | Response on violation |
|---|---|---|---|
| Request body size cap | `RESILIENT_MCP_MAX_BODY_BYTES` | 10 MiB (`10 * 1024 * 1024`) | `413 Payload Too Large` |
| Per-request compute/compile timeout | `RESILIENT_MCP_TIMEOUT_SECS` | 10 seconds | `504 Gateway Timeout` |
| Per-IP rate limit | `RESILIENT_MCP_RATE_LIMIT_PER_MIN` | 100 requests/minute/IP | `429 Too Many Requests` |

### Concurrency and logging (Phase 1, RES-3934/3937/3941)

| Behavior | Env var | Default |
|---|---|---|
| Bounded connection worker pool | `RESILIENT_MCP_MAX_CONNECTIONS` | 16 concurrent connections |

**Concurrency (RES-3937).** `run_http` accepts connections on the main
thread and hands each one to a bounded pool of worker threads over a
`sync_channel`. Once every worker (and the channel's queue slot) is busy,
the accept loop backpressures — new connections wait rather than being
dropped — instead of a single slow request blocking every other client,
which is what a sequential accept loop does.

**Access logging (RES-3941).** Every HTTP request (including ones
rejected for size/rate-limit reasons) emits one structured line to
stderr:

```
ts_ms=1737331200000 peer=127.0.0.1 method=POST path=/mcp/call status=200 duration_ms=42 bytes=128
```

Fields: `ts_ms` (Unix epoch milliseconds), `peer` (client IP), `method`,
`path`, `status` (HTTP status code), `duration_ms` (request handling
time), `bytes` (request body size). `key=value` formatting keeps it both
human-scannable in a terminal and easy for a log shipper to parse.


The body-size check inspects `Content-Length` (and the bytes actually
read) before the payload is fully buffered, so an oversized request is
rejected without allocating memory for the whole body. The compute
timeout races tool execution (parsing, typechecking, running, verifying,
...) against the configured wall-clock limit on a worker thread, so a
pathological-but-syntactically-valid program cannot hang a connection
past the deadline. The rate limiter is a token-bucket per source IP,
implemented in the in-tree [`hardening`](../resilient/src/hardening.rs)
module (no new dependencies).

### Metrics (Phase 3, RES-3952)

`GET /metrics` returns in-tree, atomic-counter-based metrics in
[Prometheus text exposition format](https://prometheus.io/docs/instrumenting/exposition_formats/)
(no new dependency — same "plain stdlib primitives" approach as the
rate limiter and access log):

```
# HELP resilient_mcp_requests_total Total HTTP requests handled, by status code.
# TYPE resilient_mcp_requests_total counter
resilient_mcp_requests_total{status="200"} 42
resilient_mcp_requests_total{status="429"} 3
# HELP resilient_mcp_request_duration_seconds HTTP request handling duration.
# TYPE resilient_mcp_request_duration_seconds histogram
resilient_mcp_request_duration_seconds_bucket{le="0.005"} 10
...
resilient_mcp_request_duration_seconds_bucket{le="+Inf"} 45
resilient_mcp_request_duration_seconds_sum 1.234000
resilient_mcp_request_duration_seconds_count 45
# HELP resilient_mcp_rate_limited_total Requests rejected by the per-IP rate limiter.
# TYPE resilient_mcp_rate_limited_total counter
resilient_mcp_rate_limited_total 3
# HELP resilient_mcp_in_flight_requests HTTP requests currently being handled.
# TYPE resilient_mcp_in_flight_requests gauge
resilient_mcp_in_flight_requests 1
```

`requests_total` is a counter keyed by `status`. Duration is a fixed-bucket
histogram (`0.005s`–`10s`, matching common Prometheus client defaults) fed
by every request regardless of outcome. `rate_limited_total` counts only
`429` rejections from the per-IP limiter (RES-3938), distinct from `413`
body-size rejections. `in_flight` is a live gauge tracked via an RAII guard
so it's accurate even on early-exit (413/429) responses. Counters are
process-local — restart resets them, same as any single-process exporter
without a pushgateway.

Example: lower every limit for a locked-down deployment:

```sh
RESILIENT_MCP_MAX_BODY_BYTES=1048576 \
RESILIENT_MCP_TIMEOUT_SECS=5 \
RESILIENT_MCP_RATE_LIMIT_PER_MIN=30 \
rz mcp --http-port 8080
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
