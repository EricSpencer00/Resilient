---
id: RES-187
title: LSP: semantic tokens for accurate syntax highlighting
state: OPEN
priority: P3
goalpost: G17
created: 2026-04-17
owner: executor
---

## Summary
Editors can fall back to TextMate grammars for highlighting, but
semantic tokens give us much better accuracy (e.g. coloring a
function call differently from a struct literal of the same name).
Provide full-file semantic tokens; delta is a follow-up.

## Acceptance criteria
- `Backend::semantic_tokens_full` returns a `SemanticTokens`
  response.
- Token types implemented (subset of the standard LSP list):
  `keyword`, `function`, `variable`, `parameter`, `type`, `string`,
  `number`, `comment`, `operator`.
- Modifiers: `declaration` on defining sites, `readonly` for
  contract / const bindings.
- Integration test exercising a small program with each token
  type represented.
- Commit message: `RES-187: LSP semantic tokens (full)`.

## Notes
- The encoded integer-array format is finicky — reference the
  spec's section on encoding and test encoding + decoding in a
  unit test, separately from the integration test.
- Don't rush a `full/delta` path; many clients only use full
  anyway.

## Log
- 2026-04-17 created by manager
