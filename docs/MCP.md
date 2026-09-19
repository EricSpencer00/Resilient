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
curl http://127.0.0.1:8080/v1/health
curl -s http://127.0.0.1:8080/v1/mcp/call \
  -H 'content-type: application/json' \
  -d '{"tool":"rz_format","input":{"source":"fn f(int x)->int{x+1}"}}'
```

The wrapper exposes versioned `GET /v1/health`, `GET /v1/readyz`,
`GET /v1/metrics`, and `POST /v1/mcp/call` routes. The original unversioned
paths remain compatibility aliases. Tool names may use
the hosted aliases from RES-3782 (`rz_compile`, `rz_format`, `rz_verify`,
and related `rz_*` names) or the native MCP names (`resilient_compile`,
`resilient_format`, `resilient_verify`, ...).

New integrations should use the `/v1` namespace. It is the stable API
contract for this HTTP wrapper; a future incompatible contract will use a new
namespace such as `/v2`. The unversioned aliases are retained temporarily so
existing clients can migrate without a flag-day change.

### Opt-in streamed tool calls (RES-3960)

Clients handling long-running calls can send `Accept: application/x-ndjson` to
`POST /v1/mcp/call` (or its unversioned compatibility alias). The server
responds with HTTP chunked transfer and writes two newline-delimited JSON
records: a `progress` record immediately before dispatch, then a `result`
record containing the normal MCP HTTP response under `result`.

```sh
curl -N http://127.0.0.1:8080/v1/mcp/call \
  -H 'Accept: application/x-ndjson' \
  -H 'content-type: application/json' \
  -d '{"tool":"rz_compile","input":{"source":"println(42)"}}'
```

The final record includes `http_status` for the status the buffered JSON
endpoint would have returned. Once streaming begins, the HTTP status line is
`200 OK`; clients should use `http_status` and the nested response status for
the final outcome. Requests without this `Accept` value keep the ordinary
single JSON response.

The HTTP wrapper is not a sandbox and is unauthenticated unless
`RESILIENT_MCP_API_KEY` is configured. Read the
[MCP HTTP security posture](MCP_SECURITY.md) before binding it beyond a
trusted local or private network; it covers authentication, TLS, exposed
execution capabilities, limits, and deployment isolation.

### `GET /v1/metrics`

Returns process-local request counters and a Prometheus text-format latency
histogram. The endpoint reports total requests, 4xx/5xx responses, cumulative
latency buckets through 10 seconds, total latency, and request count. Metrics
are reset when the MCP HTTP process restarts. The unversioned `/metrics` path
is retained as a compatibility alias.

### `GET /v1/readyz`

Returns `200 OK` when the build includes the Z3 verification backend and
`503 Service Unavailable` otherwise. Use this readiness probe for scheduler
orchestrator routing; `/v1/health` remains a liveness check that only reports
whether the HTTP process is accepting connections. The unversioned `/readyz`
and `/health` paths remain compatibility aliases.

### Hardening (Phase 1, RES-3934/3935/3936/3938/3944)

The HTTP wrapper enforces three limits, all configurable via environment
variables, all with sane defaults so a bare `rz mcp --http-port` stays
safe out of the box:

| Limit | Env var | Default | Response on violation |
|---|---|---|---|
| Request body size cap | `RESILIENT_MCP_MAX_BODY_BYTES` | 10 MiB (`10 * 1024 * 1024`) | `413 Payload Too Large` |
| Per-request compute/compile timeout | `RESILIENT_MCP_TIMEOUT_SECS` | 10 seconds | `504 Gateway Timeout` |
| Per-IP rate limit | `RESILIENT_MCP_RATE_LIMIT_PER_MIN` | 100 requests/minute/IP | `429 Too Many Requests` |
| Optional API-key authentication | `RESILIENT_MCP_API_KEY` | unset (disabled) | `401 Unauthorized` |

### Concurrency, logging, and shutdown (Phase 1, RES-3934/3937/3941/3942)

| Behavior | Env var | Default |
|---|---|---|
| Bounded connection worker pool | `RESILIENT_MCP_MAX_CONNECTIONS` | 16 concurrent connections |
| Shutdown drain grace period | `RESILIENT_MCP_SHUTDOWN_DRAIN_SECS` | 30 seconds |
| CORS allow origin | `RESILIENT_MCP_CORS_ORIGIN` | `*` |

Every HTTP response includes `Access-Control-Allow-Origin`, allowed methods
(`GET, POST, OPTIONS`), and the `Content-Type` allowed header. `OPTIONS
/v1/mcp/call` and `OPTIONS /v1/health` return a `204 No Content` preflight
response; the same applies to their unversioned compatibility aliases.
Set `RESILIENT_MCP_CORS_ORIGIN` to the exact browser origin when the service
should not be open to every origin; values containing line breaks are ignored.

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
ts_ms=1737331200000 peer=127.0.0.1 method=POST path=/v1/mcp/call status=200 duration_ms=42 bytes=128
```

Fields: `ts_ms` (Unix epoch milliseconds), `peer` (client IP), `method`,
`path`, `status` (HTTP status code), `duration_ms` (request handling
time), `bytes` (request body size). `key=value` formatting keeps it both
human-scannable in a terminal and easy for a log shipper to parse.

**Graceful shutdown (RES-3942).** On Unix, the server installs a SIGTERM
handler. On receipt, it stops accepting new connections, waits for
in-flight requests to finish (up to `RESILIENT_MCP_SHUTDOWN_DRAIN_SECS`,
default 30s), then exits `0`. This lets container orchestrators (Docker,
Kubernetes, systemd) restart or scale the service without dropping
requests that were already in flight. See
[MCP_DEPLOYMENT.md](MCP_DEPLOYMENT.md) for the full behavior writeup.

The body-size check inspects `Content-Length` (and the bytes actually
read) before the payload is fully buffered, so an oversized request is
rejected without allocating memory for the whole body. The compute
timeout races tool execution (parsing, typechecking, running, verifying,
...) against the configured wall-clock limit on a worker thread, so a
pathological-but-syntactically-valid program cannot hang a connection
past the deadline. The rate limiter is a token-bucket per source IP,
implemented in the in-tree [`hardening`](../resilient/src/hardening.rs)
module (no new dependencies).

Example: lower every limit for a locked-down deployment:

```sh
RESILIENT_MCP_MAX_BODY_BYTES=1048576 \
RESILIENT_MCP_TIMEOUT_SECS=5 \
RESILIENT_MCP_RATE_LIMIT_PER_MIN=30 \
rz mcp --http-port 8080
```

### Optional API-key authentication (RES-3939)

Set `RESILIENT_MCP_API_KEY` to require the exact value in an `X-API-Key`
header on every non-preflight HTTP route, including health and metrics
endpoints:

```sh
RESILIENT_MCP_API_KEY='replace-with-a-secret' \
rz mcp --http-port 127.0.0.1:8080

curl -s http://127.0.0.1:8080/v1/health \
  -H 'X-API-Key: replace-with-a-secret'
```

The unversioned `/health` alias also accepts the key for backward
compatibility.

Missing and invalid keys both return `401 Unauthorized` with the same generic
error body. An unset or empty variable disables the check for backwards
compatibility. API keys do not provide encryption: use HTTPS or a private
network, keep the key out of URLs and logs, and prefer a reverse proxy for
rotation and per-user authorization.

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
  "source": "fn div(int x, int y) -> int\n  requires y != 0\n{ x / y }",
  "contracts": true
}
```

The optional `contracts` boolean defaults to `true`. Set it to `false` to
skip proof checking while still validating the source. HTTP responses for
this tool include `proof_status` (`proved`, `skipped`, `unavailable`, or
`failed`) alongside the normal output fields.

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
