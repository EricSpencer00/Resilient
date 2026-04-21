---
id: RES-276
title: "lexer_logos.rs: redundant c.unwrap() inside nibble() closure match arms"
state: OPEN
priority: P4
goalpost: G3
created: 2026-04-20
owner: executor
claimed-by: Claude
---

## Summary

`resilient/src/lexer_logos.rs` contains a `nibble()` closure inside the `\xNN`
hex-escape handler. Each match arm already destructures `c` as `Some(...)`,
but then calls `c.unwrap()` to re-extract the value:

```rust
let nibble = |c: Option<char>| -> Option<u8> {
    match c {
        Some('0'..='9') => Some(c.unwrap() as u8 - b'0'),
        Some('a'..='f') => Some(c.unwrap() as u8 - b'a' + 10),
        Some('A'..='F') => Some(c.unwrap() as u8 - b'A' + 10),
        _ => None,
    }
};
```

`c.unwrap()` here can never panic (the arm is only reached when `c` matches
`Some(...)`) but it is still an `.unwrap()` call in production lexer code and
could confuse future readers or fuzz harnesses that instrument `unwrap` calls.

The idiomatic Rust fix is to bind the inner value in the match arm:

```rust
let nibble = |c: Option<char>| -> Option<u8> {
    match c {
        Some(d @ '0'..='9') => Some(d as u8 - b'0'),
        Some(d @ 'a'..='f') => Some(d as u8 - b'a' + 10),
        Some(d @ 'A'..='F') => Some(d as u8 - b'A' + 10),
        _ => None,
    }
};
```

This removes three `unwrap()` calls from production code while keeping
identical behaviour. Note: clippy currently does not flag this pattern
(the `Some(range) => Some(x.unwrap())` idiom is not a lint as of 1.80),
so the change must be made manually.

## Affected code

`resilient/src/lexer_logos.rs` — `nibble` closure, approximately lines 248–255.

## Acceptance criteria

- The three `c.unwrap()` calls inside `nibble()` are replaced with
  `d`-binding match patterns (`Some(d @ '0'..='9') => Some(d as u8 - b'0')`
  etc.).
- No observable change in lexer output for any input.
- Existing lexer unit tests in `lexer_logos.rs` pass unchanged.
- `cargo test` passes with 0 failures.
- `cargo clippy --all-targets -- -D warnings` clean.
- Commit: `RES-276: lexer_logos nibble() — bind inner char directly, drop c.unwrap()`.

## Notes

- This is a cosmetic/defensive refactor with zero behaviour change.
- The `nibble` closure is only used inside the `\xNN` branch of the
  bytes-literal lexer; its output type `Option<u8>` is preserved.
- Do NOT change any test or golden file.

## Log

- 2026-04-20 created by analyzer (lexer_logos.rs nibble() closure uses
  c.unwrap() after already matching Some(...); three redundant unwraps in
  production lexer code violate the no-panic policy in CLAUDE.md)
