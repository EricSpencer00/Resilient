---
id: RES-118
title: Parser errors include "expected one of …" hint lists
state: OPEN
priority: P3
goalpost: G6
created: 2026-04-17
owner: executor
---

## Summary
`Parser error: unexpected token RBrace` is technically accurate and
useless for someone learning the language. The Pratt parser always
knows what tokens would have been legal at that position — we just
don't thread it through to the error message. Fix by passing a
`&[TokenKind]` of expected alternatives to the error constructor.

## Acceptance criteria
- `Parser::error_expected(&self, expected: &[TokenKind], got: Token)`
  formats: `expected one of <Semicolon, Comma, RBrace>, got RBracket`.
- Every call site in `parse_*` that currently bails with `expected X,
  got Y` migrates to use the new helper, even if `expected` is a
  single-element slice.
- Missing-semicolon is the most common case — make sure
  `expected one of Semicolon` reads naturally. Override with
  `expected ;` as a specialization when the slice is just `[Semicolon]`.
- Unit tests covering three representative parser errors.
- No new parser features or recovery logic — this is pure message
  quality.
- Commit message: `RES-118: parser "expected one of …" hints`.

## Notes
- Don't try to spit out the *full* FIRST-set at deep rules — cap the
  displayed alternatives at 5 and append `…` for the rest.
- The hints should echo user-facing syntax (`;` not `Semicolon`,
  `{` not `LBrace`) via a `TokenKind::display_syntax()` method.

## Log
- 2026-04-17 created by manager
