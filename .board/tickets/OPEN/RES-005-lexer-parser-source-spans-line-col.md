---
id: RES-005
title: Lexer and parser track source spans (line, column)
state: OPEN
priority: P1
goalpost: G4
created: 2026-04-16
owner: executor
---

## Summary
Error messages today say "Parser error: Expected '(' after function
name 'main'" with no file:line:column. For a language that sells
itself on verifiability, diagnostics are the first thing a user sees
when a program is wrong — they need to be concrete. Step one: make
every token and every AST node carry a source span.

## Acceptance criteria
- `Lexer` tracks `line` and `column` (1-indexed); every token returned
  carries its starting `(line, column)` position
- Parser errors include `file:line:col` prefix, e.g.
  `hello.rs:2:14: Expected '(' after function name 'main'`
- A snippet of the offending line with a caret under the column is
  printed after the error
- Unit tests verify the lexer reports correct line/column for a
  multi-line input (e.g. token on line 3 column 5 has
  `span.line == 3, span.column == 5`)
- The existing REPL keeps working — spans in the REPL use the pseudo
  filename `<repl>`

## Notes
- Introduce a `Span { line: usize, column: usize }` struct, or a
  `(line, column)` tuple as a first step.
- Wrap `Token` as a `(Token, Span)` tuple in the lexer's output, or
  add a `Spanned<T>` wrapper. Pick the option with the smallest diff —
  you can refactor more in G6.
- Diagnostics module is fine but overkill for this ticket. Keep it
  inline.

## Log
- 2026-04-16 created by session 0
