---
id: RES-196
title: Bootstrap experiment: self-host the lexer in Resilient
state: OPEN
priority: P3
goalpost: G20
created: 2026-04-17
owner: executor
---

## Summary
G20 (self-hosting) is a long arc. The minimal demo is writing the
lexer in Resilient itself and running it against example programs.
This ticket is a *prototype* — not a replacement for the Rust
lexer, but proof that the language can express it.

## Acceptance criteria
- New file `self-host/lex.rs` (at repo root, not inside the
  `resilient/` crate).
- Implements a byte-level lexer producing the same Token stream
  as the Rust reference for a restricted subset (identifiers,
  integers, operators, the keywords we've got).
- Output format is a list of `Token { kind: String, lexeme: String, line: Int, col: Int }`.
- Tests via a driver script:
  - Run `self-host/lex.rs` on `examples/hello.rs` via the Rust
    compiler / interpreter.
  - Compare output to a reference `hello.tokens.txt` snapshot.
- README gets a section "Self-hosting progress" listing what's
  ported.
- Commit message: `RES-196: self-hosted lexer prototype`.

## Notes
- Many Resilient features used here will be missing (e.g. match
  on chars if RES-162 hasn't landed). Write around limits; add
  TODOs where the language needs to grow.
- Not in CI — this is informative only until the self-hosted
  toolchain becomes load-bearing.

## Log
- 2026-04-17 created by manager
