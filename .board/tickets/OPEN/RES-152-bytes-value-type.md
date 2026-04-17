---
id: RES-152
title: `Bytes` value type with `b"..."` literal, `len`, `slice`, `byte_at`
state: OPEN
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
