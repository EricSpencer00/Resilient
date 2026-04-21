---
id: RES-286
title: "parse_type_annotation does not accept `_` — type-hole parser arm missing"
state: OPEN
priority: P2
goalpost: G7
created: 2026-04-20
owner: executor
---

## Summary

The RES-125 deferred-AC commit (f934b75) added `fresh_hole`, the
`Type::Var(n, Some(span))` form, and the `parse_hole_span` helper in
`infer.rs`, but never added the matching parser arm.
`parse_type_annotation` has no case for `Token::Underscore`, so `_`
used in a type annotation position falls through to the catch-all arm
and records a parse error:

```
Parser error: 1:6: Expected type name for parameter, found `_`
```

As a result:
- `fresh_hole` is effectively dead code — the encoded `"_@LINE:COL"`
  string it consumes is never produced by the parser.
- The test `infer::tests::underscore_parameter_annotation_gets_hole_var`
  **FAILS** when the codebase is compiled with `--features infer`.

## Affected code

- `resilient/src/main.rs` — `parse_type_annotation` (around line 2053):
  the match has `Token::Identifier`, `Token::LeftBracket`, and a
  catch-all `_` error arm, but no `Token::Underscore` arm.

## Acceptance criteria

- `parse_type_annotation` accepts `Token::Underscore` and returns the
  encoded annotation `format!("_@{}:{}", span.start.line, span.start.column)`.
- Each occurrence of `_` in a type position gets a distinct encoded
  string with its own source position.
- `Token::Underscore` advances past the `_` token (one `self.next_token()`
  call like the `Token::Identifier` arm).
- The existing test
  `infer::tests::underscore_parameter_annotation_gets_hole_var` passes
  under `--features infer`.
- `cargo test --features infer` passes with 0 failures attributable to
  this change (the two RES-259 lint failures are a separate ticket).
- `cargo test` (no features) continues to pass.
- `cargo clippy --all-targets -- -D warnings` clean.
- Commit: `RES-286: parse_type_annotation accepts _ as type-hole annotation`.

## Notes

- `parse_hole_span` in `infer.rs` already parses the `"_@LINE:COL"`
  format — no changes needed there.
- The span to encode is the position of the `_` token. The parser's
  `current_token` carries a span; check how other parse arms capture
  it (e.g. look at how `parse_let_statement` records `span`).
- This is the remaining parser-side work from the RES-125 deferred AC.
  The type-checking and display work (Type::Var with Option<Span>,
  "type hole at L:C" display) are already present.

## Log

- 2026-04-20 created by analyzer (parser arm missing; `--features infer`
  test `underscore_parameter_annotation_gets_hole_var` fails)
