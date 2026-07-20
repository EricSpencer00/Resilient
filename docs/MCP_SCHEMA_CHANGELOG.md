# MCP Tool Schema Changelog

Versioned record of the Resilient MCP tool surface — tool names, input
schemas, and the HTTP wrapper's request/response envelope. External
consumers (bots, IDE integrations, CI scripts) should pin against a
version below and check this file before upgrading `rz`.

## Policy

- **A new entry is added whenever a released version of `rz` changes any
  of:** a tool is added or removed, a tool's `inputSchema` gains/loses/
  renames a field or changes a field's required-ness or type, an
  `rz_*` hosted alias is added/removed/retargeted, or the HTTP
  `/mcp/call` or `/health` envelope (`McpCallRequest`, `McpCallResponse`,
  `ErrorResponse`, `HealthResponse` in [openapi.json](openapi.json))
  changes shape.
- **Additive, backward-compatible changes** (new optional tool, new
  optional input field, new `rz_*` alias) bump the minor version
  (v1 → v2) and are still listed here for traceability even though no
  consumer action is required.
- **Breaking changes** (removed tool, removed/renamed field, a field
  becoming required, a response field changing type or being removed)
  bump the major version and must be called out under a **Breaking**
  heading with a migration note.
- Entries are dated by the commit that lands the schema change, oldest
  at the bottom. Each entry lists the full current tool set so a
  consumer can diff two versions without cross-referencing prior
  entries.
- This file is the source of truth for *schema* history. Behavioral
  (non-schema) changes to a tool's output content belong in the
  project's normal commit history / CHANGELOG, not here.

---

## v1 — 2026-07-19 (initial baseline)

Baseline snapshot of the schema as of the MCP HTTP wrapper (RES-3926)
plus hardening (RES-3934 series). No prior versions exist; this is the
starting point for future diffs.

**Transport:** stdio (NDJSON, protocol version `2024-11-05`) and HTTP
(`GET /health`, `POST /mcp/call`) wrapper.

**Native tool names (17):**

| Tool | Required input fields |
|---|---|
| `resilient_parse` | `source: string` |
| `resilient_typecheck` | `source: string` |
| `resilient_run` | `source: string` |
| `resilient_lint` | `source: string` |
| `resilient_format` | `source: string` |
| `resilient_check` | `source: string` |
| `resilient_verify` | `source: string` (Z3 feature required, else "not available" message) |
| `resilient_explain_lint` | `code: string` |
| `resilient_symbols` | `source: string` |
| `resilient_hover` | `source: string`, `offset: integer` |
| `resilient_compile` | `source: string` |
| `resilient_disasm` | `source: string` |
| `resilient_vm_run` | `source: string` |
| `resilient_tla_check` | `spec: string` (optional `tlc_jar: string`) |
| `resilient_fingerprint` | `source: string` |
| `resilient_resilience_score` | `source: string` |
| `resilient_contract_infer` | `source: string` |
| `resilient_call_graph` | `source: string` |

**Hosted `rz_*` aliases (8):** `rz_compile → resilient_compile`,
`rz_format → resilient_format`, `rz_verify → resilient_verify`,
`rz_parse → resilient_parse`, `rz_typecheck → resilient_typecheck`,
`rz_run → resilient_run`, `rz_lint → resilient_lint`,
`rz_check → resilient_check`.

**HTTP envelope schemas** (see [openapi.json](openapi.json) for the
full OpenAPI 3.0.3 document):

- `HealthResponse`: required `status`, `service`, `transport`, `version`.
- `McpCallRequest`: `tool`/`name` (aliases), `input`/`arguments` (aliases).
- `McpCallResponse`: required `status`, `tool`, `mcp_tool`, `stdout`,
  `stderr`, `diagnostics`, `raw_mcp`.
- `ErrorResponse`: required `status`, `error`; optional `tool`, `mcp_tool`.

**HTTP status codes:** `200` (success or tool-level error), `400`
(malformed request / dispatch error), `404` (unsupported route), `413`
(body too large), `429` (rate limited), `504` (compute timeout).

No prior schema to diff against — this is v1.
