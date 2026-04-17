---
id: RES-197
title: `resilient fmt` subcommand: deterministic source formatter
state: OPEN
priority: P3
goalpost: tooling
created: 2026-04-17
owner: executor
---

## Summary
Every language past its infancy wants a canonical formatter.
Ship a minimal `resilient fmt` that parses, pretty-prints, and
writes back. The policies are few but fixed: 4-space indent,
braces on same line, trailing comma in multi-line arg lists,
single blank line between top-level items.

## Acceptance criteria
- New subcommand `resilient fmt [<file>...] [--check] [--stdin]`:
  - No args: formats every `*.rs` under the cwd (recursive).
  - `--check`: exit 1 if any file would change; print a unified
    diff. No file writes.
  - `--stdin`: read stdin, write formatted to stdout.
- Pretty-printer module `resilient/src/fmt.rs` walks the AST and
  emits canonical whitespace. Preserves comments (including block
  comments from RES-024) and attaches them to the next statement.
- Idempotent: `fmt` on `fmt(src)` == `fmt(src)`. Unit test asserts
  this on every `examples/` file.
- Commit message: `RES-197: resilient fmt subcommand`.

## Notes
- Don't invent policy as you go — write a one-page "style guide"
  doc alongside the formatter listing every rule and the rationale.
- Comment placement is the hardest part — lean on the spans
  (RES-077..088) to associate each comment with the nearest
  AST node at parse time.

## Log
- 2026-04-17 created by manager
