---
id: RES-117
title: VM and interpreter errors print a caret line under the offending token
state: OPEN
priority: P3
goalpost: G6
created: 2026-04-17
owner: executor
---

## Summary
With RES-091/092/116 in place, all three execution modes have
source positions. What they don't yet have is the rustc-style caret
that visually points at the bad span:

```
foo.rs:12:9: Runtime error: division by zero
   let r = a / b;
           ^^^^^
```

Do it once in a `format_diagnostic` helper and share across all
three modes (interpreter, VM, JIT).

## Acceptance criteria
- New helper `fn format_diagnostic(src: &str, span: Span, level: &str, msg: &str) -> String`
  in `resilient/src/diag.rs` (new module, compiler-side only).
- Extracts the line of text from `src` via the line table, prints it
  indented by 3 spaces, then a line of `^` chars covering
  `[span.start.col, span.end.col)`.
- Multi-line spans: render only the start line + a `(span continues on line N)` tail.
- Used by: interpreter (RES-116), VM (post RES-091/092), and
  parser errors (which already have spans).
- One new unit test per mode asserting a sample error message
  contains the `^` carets.
- Commit message: `RES-117: caret diagnostics shared by all exec modes`.

## Notes
- Tabs in source: expand to 4 spaces before computing caret width
  so the underline lines up visually. Document this.
- Don't colorize (no ANSI) — some users pipe diagnostics into
  logs or the LSP channel where escapes render as garbage.

## Log
- 2026-04-17 created by manager
