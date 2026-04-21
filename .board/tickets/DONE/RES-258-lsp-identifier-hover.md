---
id: RES-258
title: "LSP: identifier hover (RES-181b) — show fn signature / type on cursor"
state: DONE
priority: P2
goalpost: G17
created: 2026-04-20
owner: executor
Claimed-by: Claude Sonnet 4.6
---

## Summary

RES-181a shipped literal-type hover (`42` → `Int`, `"hi"` → `String`).
Identifier hover — hovering over a function name or a local variable to
show its signature or inferred type — was explicitly deferred as RES-181b
pending RES-120 landing a Hindley-Milner inference layer.

RES-120 is now DONE (`resilient/src/infer.rs` exists and is feature-gated
behind `--features infer`). The blocker is gone. This ticket tracks the
follow-up hover work.

From the RES-181 DONE ticket:

> **RES-181b** — identifier hover. Gated on RES-120 landing a
> per-function type-inference result. Once RES-120 lands, the hover
> handler can look up the identifier in the inferred env and return the
> inferred monotype as a Markdown snippet.

## Acceptance criteria

- Hovering over a **function name** at its call site (or definition)
  returns a Markdown hover with the fn's signature, e.g.:
  ```
  fn add(int a, int b) -> int
  ```
- Hovering over a **local variable identifier** returns its inferred type
  (or `"unknown"` if inference did not resolve it — do not panic or
  surface an internal error).
- Hovering over a **non-identifier** token (keyword, operator, literal)
  still returns the existing literal-type hover or `null` — the new code
  must not regress RES-181a behaviour.
- Existing `hover_literal_at` tests continue to pass unchanged.
- New unit tests in `lsp_server.rs` (feature-gated `#[cfg(feature = "lsp")]`):
  - `hover_on_fn_name_returns_signature`
  - `hover_on_local_variable_returns_type`
  - `hover_on_unknown_identifier_returns_null`
- `cargo test --features lsp` passes with 0 failures.
- `cargo clippy --all-targets --features lsp -- -D warnings` clean.
- Commit: `RES-258: LSP identifier hover — fn signatures and local types`.

## Notes

- The `infer` feature in `infer.rs` runs Algorithm W over function
  bodies. The hover handler should call into the same inference path
  and look up the result env keyed by identifier name and cursor
  position.
- If inference is unavailable (e.g. the file doesn't compile past the
  parser), return `null` rather than an error response.
- `identifier_at` (already in `lsp_server.rs`) resolves the token under
  the cursor — reuse it.
- Module-level comment at `lsp_server.rs` line 8 says "no hover" —
  update it when this lands (or as part of a separate cleanup ticket).

## Log

- 2026-04-20 created by analyzer (RES-120 done unblocks RES-181b;
  identifier hover not yet tracked in OPEN)
