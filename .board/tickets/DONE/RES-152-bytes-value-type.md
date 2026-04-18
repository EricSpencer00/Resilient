---
id: RES-152
title: `Bytes` value type with `b"..."` literal, `len`, `slice`, `byte_at`
state: DONE
priority: P3
goalpost: G11
created: 2026-04-17
owner: executor
---

## Summary
Embedded work is byte-oriented: protocol frames, register maps,
packed structs on the wire. Strings are UTF-8-only and not the
right primitive. Add a dedicated `Bytes` value type so users don't
reach for `Array<Int>` (which is a Vec<i64> under the hood — 8×
the memory).

## Acceptance criteria
- `Value::Bytes(Vec<u8>)` (std) / `Bytes(alloc::vec::Vec<u8>)`
  (no_std alloc).
- Literal: `b"\x00\x01\x7f"` — hex escapes required for
  non-printable bytes. Unicode escapes are disallowed (this is a
  byte literal, not a string).
- Builtins: `bytes_len(b) -> Int`, `bytes_slice(b, start, end) ->
  Bytes`, `byte_at(b, i) -> Int` (returns 0..255).
- Typechecker adds `Type::Bytes`; unify rules mirror `String` but
  distinct.
- Unit tests: literal parsing with all three escape forms, slice
  out-of-range errors with span, byte_at in bounds + out of
  bounds.
- Commit message: `RES-152: Bytes value type + builtins`.

## Notes
- `byte_at` returns Int (i64), not a narrower type — we don't have
  `u8` at the Resilient level, and narrowing belongs to a future
  fixed-width-int ticket.
- No conversion between String and Bytes in this ticket — follow-up
  for `to_bytes(s)` / `from_bytes(b)` once we settle encoding
  semantics.

## Log
- 2026-04-17 created by manager
- 2026-04-17 claimed by executor
- 2026-04-17 done by executor

## Resolution
- `resilient/src/main.rs`:
  - New `Token::BytesLiteral(Vec<u8>)` variant, with display
    `"bytes literal"`.
  - New `Node::BytesLiteral { value: Vec<u8>, span }` AST variant.
  - New `Value::Bytes(Vec<u8>)` variant. `Display` round-trips
    through a `b"..."` literal with hex escapes (`\xNN`) for
    non-printables and the five named escapes
    (`\n` / `\t` / `\r` / `\\` / `\"`) for readability.
  - Hand-rolled lexer: `'b' if self.peek_char() == '"'` dispatch
    calls a new `read_bytes()` that walks the literal, decoding:
    - named: `\n` / `\t` / `\r` / `\0` / `\\` / `\"`
    - hex: `\xNN` — exactly two hex digits, malformed pairs
      pass through as literal `\xHH`
    - printable ASCII: verbatim
    - non-ASCII chars: UTF-8 bytes of the char
    - unknown escapes (including `\u{...}`): passed through as
      literal `\` + following char — the ticket's "Unicode
      escapes are disallowed" is honored by **not interpreting**
      `\u`, so a user writing `b"\u{41}"` gets six literal
      bytes, not `b"A"`.
  - Parser arm `Token::BytesLiteral(v)` → `Node::BytesLiteral`.
  - Eval arm `Node::BytesLiteral` → `Value::Bytes(value.clone())`.
  - Three new builtins registered in the `BUILTINS` table:
    - `bytes_len(b) -> Int`
    - `bytes_slice(b, start, end) -> Bytes` — half-open
      `[start, end)`, rejects negative indices, `start > end`,
      and `end > len` with distinct diagnostics.
    - `byte_at(b, i) -> Int` — returns `0..=255`, rejects
      negative and out-of-range indices.
- `resilient/src/lexer_logos.rs`:
  - New `BytesLit(Vec<u8>)` tok + `bytes_lit` callback mirroring
    the hand-rolled decoder. Priority bumped to 3 so `b"..."`
    wins over `Ident("b") Str(...)`.
  - `convert` arm maps `Tok::BytesLit(b)` → `Token::BytesLiteral(b)`.
- `resilient/src/typechecker.rs`:
  - New `Type::Bytes` variant (Display `"bytes"`). Unify rules
    mirror `String` but the two are distinct — `String` and
    `Bytes` don't interchange.
  - `Node::BytesLiteral` arm added to exhaustive `check_node`.
  - Three `bytes_*` builtins registered with precise
    `(Bytes, …) -> …` signatures.
- `resilient/src/unify.rs`: `Type::Bytes` added to the apply
  impl's primitive-passthrough list.
- `resilient/src/compiler.rs`: `Node::BytesLiteral` arm added to
  `node_line`'s exhaustive span accessor.
- Deviations from ticket:
  - "span on slice out-of-range errors" — builtin layer emits
    the diagnostic string; span wrapping happens at the
    interpreter call-site (same precedent as every other
    RES-0xx builtin runtime error, e.g. file_read).
  - `\u` rejection is implemented as "don't interpret" rather
    than "hard error". The test
    `bytes_literal_treats_unicode_escape_as_literal` locks down
    that `b"\u{41}" != b"A"` — the user cannot accidentally
    rely on Unicode semantics. A hard-error would require
    lex-level error plumbing that doesn't exist today (lexer
    has no error list); a future ticket could tighten if
    needed.
- Unit tests (10 new):
  - `bytes_literal_hex_named_and_printable_escapes` — all three
    escape forms in one literal, decoded round-trips to the
    expected bytes.
  - `bytes_literal_treats_unicode_escape_as_literal` — verifies
    `\u{...}` is NOT interpreted as a Unicode escape.
  - `bytes_len_counts_bytes` / `bytes_len_rejects_non_bytes`
  - `bytes_slice_returns_new_bytes`
  - `bytes_slice_rejects_out_of_range` — negative index,
    `start > end`, and `end > len` branches
  - `byte_at_in_bounds_returns_int_0_to_255` — exercises 0,
    128, 255 and asserts the returned Int is in `0..=255`
  - `byte_at_out_of_bounds_errors` — `i >= len` and negative
  - `byte_at_rejects_non_bytes_first_arg`
  - `bytes_display_roundtrips_through_hex_escapes` — Display
    produces a parseable `b"..."` literal
- Smoke (manual):
  - `let b = b"\x00\x01Hello\x7f"` prints `b"\x00\x01Hello\x7f"`,
    `bytes_len(b)` = 8, `byte_at(b, 2)` = 72 (`'H'`),
    `bytes_slice(b, 2, 7)` = `b"Hello"`.
- Verification:
  - `cargo test --locked` — 400 passed (was 390 before RES-152)
  - `cargo test --locked --features logos-lexer` — 401 passed
  - `cargo clippy --locked --features logos-lexer,z3 --tests
    -- -D warnings` — clean (after collapsing two let-chains)
