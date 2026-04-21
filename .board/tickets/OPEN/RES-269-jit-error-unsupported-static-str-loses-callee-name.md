---
id: RES-269
title: "JIT: JitError::Unsupported(&'static str) loses callee name in 'call to unknown function' diagnostic"
state: OPEN
priority: P4
goalpost: G3
created: 2026-04-20
owner: executor
---

## Summary

`JitError::Unsupported` takes a `&'static str`, which means dynamic context
(such as the callee function name) cannot be embedded in the error message.
The comment at `jit_backend.rs` line 1677 explicitly notes this:

```rust
// Note: we lose the actual name in the
// diagnostic since JitError::Unsupported
// takes &'static str. A richer diagnostic
// type is a future ticket.
return Err(JitError::Unsupported("call to unknown function"));
```

When a Resilient program calls a function the JIT hasn't seen (e.g. a
builtin that isn't yet lowered), the user sees `jit: unsupported: call to
unknown function` with no indication of which function triggered the error.

## Affected code

`resilient/src/jit_backend.rs`:
- `enum JitError` (line ~487) — `Unsupported(&'static str)` variant
- `fn lower_expr` (line ~1673) — the "call to unknown function" path
- `impl Display for JitError` (line ~499)

## Acceptance criteria

- `JitError::Unsupported` is changed to carry a `String` (or `Cow<'static, str>`)
  so dynamic context can be embedded.
- The "call to unknown function" path in `lower_expr` includes the callee
  name: `JitError::Unsupported(format!("call to unknown function: `{}`", callee_name))`.
- Other existing `JitError::Unsupported("...")` call sites that use string
  literals continue to compile (wrap in `String::from(...)` or use `into()`).
- `Display` impl and test assertions that match on `"jit: unsupported: ..."` are
  updated to match the new string shape.
- `cargo test` passes with 0 failures.
- `cargo clippy --all-targets -- -D warnings` clean.
- Commit: `RES-269: JitError::Unsupported — carry String for richer diagnostics`.

## Notes

Changing `&'static str` to `String` is a minor breaking change inside the
`JitError` type; since `JitError` is not part of the stable public API
(it's compiler-internal), this is safe to do without a stability notice.

The simplest fix is:
```rust
enum JitError {
    Unsupported(String),
    // ...
}
```
and then `JitError::Unsupported("literal".into())` at each call site.

## Log

- 2026-04-20 created by analyzer (comment at jit_backend.rs:1677 explicitly
  flags this as a future ticket)
