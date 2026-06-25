# Resilient MCP Server - Deployment Decision

## Decision

Ship the Resilient MCP server as a containerized HTTP service, then deploy the
same image to the cheapest reliable host that can run the native `rz` binary
and Z3.

Preferred order:

1. **Railway or Fly.io for staging**: fastest path from repo image to a public
   endpoint with logs, restart policy, and HTTPS.
2. **Cloudflare Containers if Cloudflare is desired**: viable for this native
   Rust/Z3 workload because it runs the existing Linux container image.
3. **VPS or sponsor-backed VM for long-running production**: best predictable
   monthly cost once usage is real.
4. **AWS free tier only for temporary experiments**: useful for a short trial,
   but avoid depending on rotating free-tier accounts for a public endpoint.

Do not target plain Cloudflare Workers for the compiler process. Workers are a
good edge proxy, but the MCP service needs a native binary, process lifetime,
and solver/runtime headroom that fit a container or VM much better.

## Local HTTP Wrapper

The compiler now exposes a hosted transport without adding a web framework:

```sh
rz mcp --http-port 8080
```

Equivalent flag form:

```sh
rz --mcp-http-port 8080
```

Health check:

```sh
curl http://127.0.0.1:8080/health
```

Tool call:

```sh
curl -s http://127.0.0.1:8080/mcp/call \
  -H 'content-type: application/json' \
  -d '{"tool":"rz_format","input":{"source":"fn f(int x)->int{x+1}"}}'
```

The HTTP wrapper accepts hosted aliases such as `rz_compile`, `rz_format`,
and `rz_verify`, and maps them onto the existing MCP tools.

## Container Command

The existing Docker image has `rz` as its entrypoint:

```sh
docker run --rm -p 8080:8080 ghcr.io/ericspencer00/resilient:latest \
  mcp --http-port 8080
```

For source builds:

```sh
docker build -t resilient-mcp .
docker run --rm -p 8080:8080 resilient-mcp mcp --http-port 8080
```

## Platform Notes

| Platform | Fit | Notes |
| --- | --- | --- |
| Railway | Good first deploy | Git/image deploys, logs, restart policy, simple HTTPS endpoint. |
| Fly.io | Good first deploy | Similar container-first workflow; easier regional placement. |
| Cloudflare Containers | Good Cloudflare path | Use the container product, not Workers-only, so `rz` and Z3 run as native Linux processes. |
| AWS EC2/Lightsail | Fine but ops-heavy | Free credits are useful for staging; production needs billing, patching, and monitoring discipline. |
| VPS/sponsor VM | Best long-term cost | Use systemd or a container runtime plus external uptime monitoring. |

## HTTP API

### `GET /health`

Returns:

```json
{
  "status": "ok",
  "service": "resilient-mcp",
  "transport": "http",
  "version": "<rz version>"
}
```

### `POST /mcp/call`

Request:

```json
{
  "tool": "rz_compile",
  "input": {
    "source": "println(42)"
  }
}
```

Response:

```json
{
  "status": "ok",
  "tool": "rz_compile",
  "mcp_tool": "resilient_compile",
  "stdout": "...",
  "stderr": "",
  "diagnostics": [],
  "raw_mcp": {}
}
```

## Production Checklist

- Put HTTPS and auth in front of the service before advertising a public URL.
- Limit request bodies at the proxy; the server has a 10 MiB request cap.
- Set provider-level request timeouts and restart policy.
- Monitor `GET /health` from outside the provider.
- Keep Z3 installed in the runtime image for verifier-backed tools.
- Start with `rz_format`, `rz_compile`, and `rz_verify`; add auth/rate limits
  before opening broader tool access.

## First Deployment Target

Use Railway or Fly.io if the goal is fastest public staging. Use Cloudflare
Containers if the project specifically wants Cloudflare ownership and accepts
the extra Worker/Durable Object routing layer. Ask for sponsorship, or move to
a small VPS, once there is enough usage to justify a permanent endpoint.
