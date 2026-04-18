---
id: RES-117
title: VM and interpreter errors print a caret line under the offending token
state: DONE
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
- 2026-04-17 claimed by executor
- 2026-04-17 done by executor

## Resolution

Files added:
- `resilient/src/diag.rs` (new) — `format_diagnostic(src, span,
  level, msg)` renders the rustc-style source-context block
  (level + msg + indented source line + caret underline, with
  a `(span continues on line N)` tail for multi-line spans and
  tab expansion to 4 spaces so the underline lines up
  visually). Plus a convenience
  `format_diagnostic_from_line_col(src, line, col, level, msg)`
  that builds a zero-width span at `(line, col)` for callers
  that only have a `<line>:<col>:` string prefix to work with.
  Six unit tests — three shaped like each execution mode's
  error string (interpreter div-by-zero, VM `AtLine`, parser
  bare-prefix), plus multi-line, tab expansion, and
  out-of-range defensive coverage.

Files changed:
- `resilient/src/main.rs`
  - `mod diag;` declaration.
  - New `render_with_caret(src, err, level) -> String` helper
    that parses a `line:col:` or `path:line:col:` prefix,
    strips any duplicate `level:` from the payload, and appends
    the caret block via `diag::format_diagnostic_from_line_col`.
  - Wired into `execute_file`'s three error paths:
    - **Parser** errors: on `parser.errors.is_empty() == false`,
      iterates the collected strings and `eprintln!`s each
      through `render_with_caret`. The parser's own inline
      `record_error` `eprintln!` still fires (it's the
      low-latency path), keeping the diff small.
    - **Typechecker** errors: after the existing red-ANSI
      header, prints the caret block underneath.
    - **VM** errors: `VmError::AtLine { line, kind }` gets a
      caret block via `format_diagnostic_from_line_col(contents,
      line, 1, ...)`. Column defaults to 1 because the VM only
      tracks line today — precise column plumbing is a follow-up
      against RES-091's `AtLine` struct, not RES-117.
    - **Interpreter** errors: the `format_interpreter_error`
      header now gets wrapped with `render_with_caret` before
      the driver returns the error to `main`.

Deviation from the ticket:

- `format_diagnostic`'s `Span` parameter is the exact signature
  the ticket requested; `format_diagnostic_from_line_col` is an
  addition, not a replacement. The driver uses the convenience
  form because the existing error pipeline threads `line:col:`
  strings rather than real `Span`s — promoting the pipeline to
  carry `Span`s end-to-end is substantial plumbing (a thread-
  `Span`-everywhere refactor) and belongs in its own ticket.
  The helper itself is span-based; the convenience is a shim.
- No ANSI colour in `diag.rs` — per the ticket's note.

Verification:
- `cargo build --locked` — clean.
- `cargo test --locked` — 278 unit (+6 new `diag::tests`) + 3
  dump-tokens + 12 examples-smoke + 1 golden pass.
- `cargo test --locked --features logos-lexer` — 279 unit pass.
- `cargo clippy --locked -- -D warnings` — clean.
- `cargo clippy --locked --tests -- -D warnings` — clean.
- Manual:
  - Interpreter: `boom(0); // div by zero` prints
    `Error: .../boom.rs:5:5: Runtime error: Division by zero`
    followed by the caret block pointing under column 5.
  - VM: `--vm` on the same source prints
    `VM runtime error: vm: divide by zero` with a caret under
    the offending line.
  - Parser: `fn main() { let = 1; }` prints the
    `Parser error: 2:9: Expected identifier after 'let', found
    Assign` header and then the caret block under `let`'s `=`.
