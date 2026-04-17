---
id: RES-010
title: Fix lexer panic on `.` outside float literals
state: OPEN
priority: P2
goalpost: G5
created: 2026-04-16
owner: executor
---

## Summary
`comprehensive.rs` panics:

```
thread 'main' panicked at src/main.rs:183:21:
Unexpected character: .
```

The lexer's default branch calls `panic!("Unexpected character: {}", ...)`.
`.` outside a number literal (e.g. field access, method call, or a
stray dot in text) crashes the binary. Should either be a lexer error
we recover from, or the `.` token should be recognized (pending field
access design decisions in G12).

## Acceptance criteria
- Lexer does not panic on `.` or any other unexpected character
- Instead: emits an error token and the parser records a parse error
  through `record_error`
- Unit test: `tokenize(". 1.5")` yields an error token followed by
  `FloatLiteral(1.5)`
- `comprehensive.rs` no longer panics — it may still error out, but
  with a diagnostic, not a process crash
- No new `panic!` calls in `Lexer::next_token` default arm

## Notes
- Site: `resilient/src/main.rs:183` (the `_ => panic!(...)` arm).
- Suggested token name: `Token::Error(char)` or just `Token::Unknown`
  so the parser can recover.
- For G12 (structs), `.` will be a real token with precedence; keep
  this fix minimal so that future work doesn't have to un-break it.

## Log
- 2026-04-16 created by manager
