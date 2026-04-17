---
id: RES-189
title: LSP: inlay hints for inferred `let` types
state: OPEN
priority: P3
goalpost: G17
created: 2026-04-17
owner: executor
---

## Summary
Once RES-123 lands (inferred return types) and RES-120
(inference), users will omit annotations. Inlay hints show the
inferred type inline as editor chrome — `let x = 3 + 2  :: Int`.

## Acceptance criteria
- `Backend::inlay_hint` returns a list of `InlayHint` values over
  a requested range.
- Emit one hint per `let` binding that lacks an explicit type
  annotation, with the inferred type as the label and position at
  the end of the pattern.
- Parameter hints: at a call site, label each positional arg with
  the corresponding param name (`add(a: 1, b: 2)`-style chrome).
  Off by default, behind a workspace config setting
  `resilient.inlayHints.parameters: bool`.
- Integration test: 5-let snippet, 3 hints expected.
- Commit message: `RES-189: LSP inlay hints for inferred types`.

## Notes
- Hints must not interfere with diagnostics — they're purely
  visual. Don't introduce any new AST passes; reuse the inference
  cache from RES-120.
- Client behavior varies on when to refresh — we respect
  `inlayHint/refresh` notifications.

## Log
- 2026-04-17 created by manager
