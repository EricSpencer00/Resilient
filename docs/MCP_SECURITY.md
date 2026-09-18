---
title: MCP HTTP Security Posture
nav_order: 12
permalink: /mcp-security
---

# MCP HTTP Security Posture

This document describes the security boundary of the hosted MCP HTTP wrapper
(`rz mcp --http-port ...`). It is an operator guide, not a promise that the
HTTP wrapper is a sandbox.

## Security summary

The HTTP wrapper is a plain-HTTP adapter around the Resilient compiler and
runtime. It is unauthenticated unless `RESILIENT_MCP_API_KEY` is configured:

- When `RESILIENT_MCP_API_KEY` is set, every non-preflight HTTP route requires
  the exact value in an `X-API-Key` header. Missing and invalid values both
  return 401; CORS preflight requests remain unauthenticated so browsers can
  discover the allowed header before sending the credentialed request.
- When the variable is unset or empty, the wrapper does not authenticate
  requests. This preserves the local/private default and is not suitable for
  a public listener.
- The `--http-port 8080` shorthand binds to `0.0.0.0:8080`. Use an explicit
  private or loopback address when the service is not intentionally public.
- TLS termination, authentication, tenant isolation, and network policy belong
  in front of the process.
- The process has the ambient authority of its operating-system user. A
  caller that can invoke `resilient_run` or `resilient_vm_run` must be treated
  as able to execute untrusted code with that authority. The standard library
  includes file, process, and network-capable operations.
- CORS is browser policy, not authentication. The default
  `RESILIENT_MCP_CORS_ORIGIN=*` does not restrict who can connect.

For a local client, prefer the stdio transport. For HTTP, keep the listener on
a private network and put an authenticated TLS reverse proxy in front of it.

## Trust boundary and assets

The trust boundary is the TCP connection at the HTTP listener. Request JSON,
Resilient source, TLA+ specifications, tool names, and optional tool paths are
untrusted until they have been parsed and validated by the server.

The deployment should protect at least these assets:

| Asset | Why it matters |
| --- | --- |
| Host filesystem, environment, and credentials | `resilient_run` and `resilient_vm_run` execute code with the process user's authority. |
| CPU, memory, threads, and file descriptors | Compilation, verification, execution, and model checking are attacker-controlled workloads. |
| Submitted source and diagnostics | Requests and tool output may contain proprietary source, paths, or contract details. |
| Z3, Java, and TLC inputs | Verification tools run external processes or consume external tool paths. |
| Service availability and metrics | Health and metrics endpoints reveal operational state and are useful to an attacker probing the service. |

The server is not a multi-tenant isolation boundary. Run separate instances,
containers, or hosts when callers must not share process, filesystem, or
resource authority.

## Exposed capabilities

The HTTP endpoint exposes the same tool registry returned by `tools/list`,
including parsing, compilation, verification, execution, VM execution,
disassembly, and TLA+ checking. In particular:

- `resilient_run` and `resilient_vm_run` execute submitted Resilient programs.
- `resilient_tla_check` writes the submitted specification to a temporary file
  and can accept a caller-supplied `tlc_jar` path. Do not expose this tool to
  untrusted callers unless the process is isolated and the allowed Java/TLC
  installation is controlled by the deployment.
- Verification and model checking can start external solver processes and
  consume substantial CPU or memory.

If a deployment needs only formatting or static analysis, enforce an
allow-list at the reverse proxy or API gateway. The wrapper itself does not
provide per-tool authorization.

## Built-in resource controls

The wrapper applies these process-local controls before or during request
handling:

| Control | Default | What it protects | Important limitation |
| --- | ---: | --- | --- |
| Request body cap (`RESILIENT_MCP_MAX_BODY_BYTES`) | 10 MiB | Memory spent buffering one HTTP request | Set a smaller proxy limit as defense in depth. |
| Tool timeout (`RESILIENT_MCP_TIMEOUT_SECS`) | 10 seconds | Client-visible wall-clock request time | The worker is abandoned when the timeout fires; execution is not forcibly killed. Use cgroups and a restart policy for hard resource containment. |
| Per-IP rate limit (`RESILIENT_MCP_RATE_LIMIT_PER_MIN`) | 100/minute | Burst volume from one observed peer address | The server does not interpret `X-Forwarded-For`; behind a proxy, configure rate limiting at the trusted edge. |
| Worker pool (`RESILIENT_MCP_MAX_CONNECTIONS`) | 16 | Concurrent HTTP connections accepted for handling | It bounds connections, not every external solver or abandoned timeout thread. |
| Shutdown drain (`RESILIENT_MCP_SHUTDOWN_DRAIN_SECS`) | 30 seconds | Graceful termination window | Send `SIGTERM` and set the orchestrator stop timeout at or above this value. |
| CORS origin (`RESILIENT_MCP_CORS_ORIGIN`) | `*` | Browser cross-origin policy | It is not an authentication control. Set one exact origin when browser access is required. |

Lower these values for a public or expensive deployment. Apply independent
limits at the proxy, container, and host: request rate, request size, CPU,
memory, process count, open files, and outbound network connections.

## Authentication and transport

The recommended deployment pattern is:

1. Bind `rz` to loopback or a private interface:

   ```sh
   rz mcp --http-port 127.0.0.1:8080
   ```

2. Terminate HTTPS at a reverse proxy or API gateway.
3. Require the gateway's established authentication mechanism (for example,
   an identity-aware proxy, mTLS, or a short-lived bearer token). The built-in
   `X-API-Key` mode is suitable for a single shared deployment credential, not
   per-user authorization or key rotation.
4. Permit only the required routes and tools, and apply proxy-side request,
   timeout, concurrency, and rate limits.
5. Do not expose the backend port directly or put bearer tokens in URLs.

If a public listener is unavoidable, use a private firewall rule or gateway
allow-list in addition to authentication. Do not treat an allow-list,
`Origin`, or CORS response header as a substitute for authentication.

## Container and host isolation

Run the service as a dedicated non-root user in a container or equivalent OS
sandbox. For an internet-facing deployment:

- use a read-only root filesystem and provide only the temporary directory
  needed by the configured tools;
- do not mount host credentials, source trees, Docker sockets, or package
  manager credentials into the container;
- apply CPU, memory, PID, and file-descriptor limits;
- restrict outbound network access unless a specific tool requires it;
- keep Java, TLC, Z3, and the `rz` image pinned and patched;
- use a restart policy and an external health check, but alert on repeated
  restarts rather than treating them as recovery from arbitrary workloads.

The root [security policy](../SECURITY.md) covers vulnerability reporting and
the broader ambient-authority model for Resilient programs.

## Data handling and observability

The built-in access log records request metadata (`peer`, method, path, status,
duration, and body byte count), not the request body. Tool responses can still
contain submitted source, diagnostics, filesystem paths, or program output.
Treat HTTP responses, reverse-proxy logs, stderr, metrics, and crash reports as
potentially sensitive:

- disable request-body logging at the proxy;
- restrict log and metrics access;
- define retention and redaction rules before accepting proprietary source;
- avoid returning secrets from executed programs;
- monitor `429`, `413`, `504`, process restarts, memory pressure, and external
  solver failures.

`/health` is a liveness endpoint and `/readyz` reports whether the optional Z3
backend is available. `/metrics` exposes process-local request counters and a
latency histogram. Keep these routes private or protect them at the edge when
their operational information is not intended for the public internet.

## Deployment checklist

Before advertising an HTTP endpoint, confirm:

- [ ] The listener is loopback/private, or an authenticated TLS edge is the
      only reachable entry point.
- [ ] Authentication and authorization are enforced before `/mcp/call`.
- [ ] The allowed tool set is smaller than the full registry when execution or
      model checking is not required.
- [ ] Proxy and process limits are set for body size, rate, timeout,
      concurrency, CPU, memory, and process count.
- [ ] The service runs non-root with no host credentials or unnecessary mounts.
- [ ] CORS is set to an exact browser origin, or browser access is disabled.
- [ ] Logs and metrics are access-controlled and do not capture request bodies.
- [ ] Solver/model-checker paths and versions are deployment-controlled.
- [ ] A health check, restart policy, and alerting are configured.
- [ ] The deployment has a documented owner and a patch/rollback procedure.
