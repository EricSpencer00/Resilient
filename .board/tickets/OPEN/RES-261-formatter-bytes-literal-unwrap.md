---
id: RES-261
title: "formatter.rs: replace infallible `unwrap()` in BytesLiteral with `char::from`"
state: OPEN
priority: P4
goalpost: tooling
created: 2026-04-20
owner: executor
---

## Summary

`resilient/src/formatter.rs` line 486 contains:

```rust
x if x.is_ascii_graphic() || x == b' ' => {
    let mut buf = [0u8; 1];
    buf[0] = x;
    self.write(std::str::from_utf8(&buf).unwrap());
}
```

`std::str::from_utf8` on a single ASCII byte (graphic chars 0x21-0x7E
or space 0x20) can never fail — every ASCII byte is valid UTF-8 — but the
`unwrap()` is present nonetheless. CLAUDE.md requires the compiler
(including the formatter) to have zero panics in production code
(all error paths must return a typed error or use infallible
conversions).

The correct replacement is `char::from(x)`, which converts a `u8` to a
`char` without allocation and is definitionally infallible for all input:

```rust
x if x.is_ascii_graphic() || x == b' ' => {
    self.write(&char::from(x).to_string());
}
```

Or equivalently using the `as char` cast:

```rust
self.write(&format!("{}", x as char));
```

## Acceptance criteria

- Line 486 in `formatter.rs` is rewritten to use `char::from(x)` (or
  `x as char`) instead of the `from_utf8(&buf).unwrap()` idiom.
- The temporary `buf` array and its assignment are removed.
- Existing formatter tests pass unchanged.
- `cargo clippy --all-targets -- -D warnings` clean.
- `cargo test` passes with 0 failures.
- Commit: `RES-261: formatter BytesLiteral — replace unwrap with char::from`.

## Notes

- This is a pure refactor with no behavioural change. The formatter's
  output for `BytesLiteral` nodes is identical before and after.
- The fix is a one-line change inside a single match arm.
- No new tests are required — the existing `fmt_bytes_literal_*` tests
  in `formatter.rs` cover this code path.

## Log

- 2026-04-20 created by analyzer (infallible `unwrap()` in production
  formatter code; violates no-panic rule in CLAUDE.md)
