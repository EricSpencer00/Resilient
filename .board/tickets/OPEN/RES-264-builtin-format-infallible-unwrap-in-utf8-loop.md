---
id: RES-264
title: "builtin_format: infallible unwrap() in UTF-8 byte loop — replace with chars().next() guard"
state: OPEN
priority: P4
goalpost: G11
created: 2026-04-20
owner: executor
---

## Summary

`builtin_format` in `resilient/src/main.rs` (line ~5949) contains an
`unwrap()` that is safe in practice but unnecessarily present:

```rust
_ => {
    // Copy one UTF-8 scalar at a time using the string
    // slice: `char_indices` would do this, but we're
    // already walking bytes. Find the next char boundary.
    let rest = &fmt[i..];
    let ch = rest.chars().next().unwrap();  // ← line 5949
    out.push(ch);
    i += ch.len_utf8();
}
```

The comment explains the intent: `rest = &fmt[i..]` and the outer loop
runs only while `i < bytes.len()`, so `rest` is never empty, and
`.chars().next()` never returns `None`. The `unwrap()` can therefore
never panic. However:

1. It violates the project's no-unwrap-in-library-code convention (same
   rule that drove RES-261 for the formatter).
2. Any reader unfamiliar with the loop invariant must reason about whether
   the unwrap is safe, adding cognitive overhead.

The clean replacement is an explicit guard that signals the invariant:

```rust
let Some(ch) = rest.chars().next() else {
    break;   // unreachable, but makes the invariant explicit
};
```

Or equivalently using `if let` with an `unreachable!()` branch is
acceptable, but the `else { break; }` form silently skips a corrupt byte
sequence rather than panicking, which is the safer embedded-systems choice.

## Affected code

`resilient/src/main.rs` — `fn builtin_format`, line ~5949.

## Acceptance criteria

- The `let ch = rest.chars().next().unwrap();` line is replaced with:
  ```rust
  let Some(ch) = rest.chars().next() else { break; };
  ```
- No other changes to `builtin_format` logic.
- All existing `builtin_format` tests continue to pass (no behaviour
  change for valid inputs; for the unreachable empty-rest case, looping
  terminates cleanly instead of panicking).
- `cargo test` passes with 0 failures.
- `cargo clippy --all-targets -- -D warnings` clean.
- Commit: `RES-264: builtin_format — replace infallible unwrap with let-else guard`.

## Notes

- This is a pure defensive refactor with no observable behaviour change
  for any input that can arise in practice (a UTF-8 string slice whose
  byte iterator has remaining bytes will always yield a char).
- The `break` in the `else` arm is unreachable but makes the control
  flow transparent to future readers.
- No new tests are required — the existing format builtin tests
  (`format_interpolates_placeholders_in_order` etc.) cover this code path.

## Log

- 2026-04-20 created by analyzer (infallible `unwrap()` in production
  `builtin_format` byte loop at main.rs:5949; violates no-unwrap rule
  from CLAUDE.md; same class of defect as RES-261)
