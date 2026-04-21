---
id: RES-263
title: "bytecode.rs: patch_jump panics on non-jump op — replace with CompileError"
state: OPEN
priority: P3
goalpost: G3
created: 2026-04-20
owner: executor
---

## Summary

`Chunk::patch_jump` in `resilient/src/bytecode.rs` (line ~196) contains
a `panic!` in its catch-all arm:

```rust
other => {
    panic!("patch_jump called on non-jump op: {:?}", other);
}
```

`patch_jump` is called directly by `compiler.rs` (14 call sites across
`compile_stmt` and `compile_expr`) and is therefore production compiler
code, not test code. CLAUDE.md states: "A panic in the compiler is a bug."

Under normal compiler operation the invariant holds (we only call
`patch_jump` on an index we just emitted as a `Jump`/`JumpIfFalse`/
`JumpIfTrue`). However:

1. Future compiler work (e.g., the JIT pipeline RES-248..250, or
   macro-like code generation) could call `patch_jump` on a wrong index,
   making the panic reachable.
2. Fuzz harnesses that feed the bytecode compiler directly (RES-256) could
   trigger this path if the fuzzer synthesizes a `Program` where
   `patch_jump` is called on a non-jump.

The fix is straightforward: return `Err(CompileError::InternalError(...))`.
`CompileError` already exists and is propagated with `?` everywhere in the
compiler.

## Affected code

`resilient/src/bytecode.rs` lines 191-198:

```rust
match &mut self.code[patch_idx] {
    Op::Jump(o) => *o = offset,
    Op::JumpIfFalse(o) => *o = offset,
    Op::JumpIfTrue(o) => *o = offset,
    other => {
        panic!("patch_jump called on non-jump op: {:?}", other);
    }
}
```

## Acceptance criteria

- A new `CompileError` variant is added:
  ```rust
  InternalError(&'static str),
  ```
  (or reuse a descriptive existing variant if appropriate).
- The `panic!` in `patch_jump`'s catch-all arm is replaced with:
  ```rust
  return Err(CompileError::InternalError("patch_jump: not a jump op"));
  ```
- `patch_jump` signature changes from `-> Result<(), CompileError>` to
  remain `-> Result<(), CompileError>` (it already returns a `Result`).
- The `Display` impl for `CompileError` in `bytecode.rs` is updated to
  render the new variant.
- All existing callers in `compiler.rs` already propagate `?` — no
  call-site changes are needed.
- New unit test in `bytecode.rs` `#[cfg(test)]`:
  `patch_jump_on_non_jump_returns_error` — emit a `Const(0)`, call
  `patch_jump(0, 1)`, assert `Err(CompileError::InternalError(_))`.
- `cargo test` passes with 0 failures.
- `cargo clippy --all-targets -- -D warnings` clean.
- Commit: `RES-263: patch_jump — replace panic with CompileError::InternalError`.

## Notes

- This is purely defensive: the invariant that callers only pass jump
  indices is still expected to hold. The change converts an impossible
  panic into an impossible error return — safer for fuzz targets and
  future code.
- The `CompileError::InternalError` variant may be useful for similar
  "this should never happen" paths in the compiler; feel free to add it
  as a general-purpose internal-invariant error type.
- Do NOT change the `Display` output of existing `CompileError` variants.

## Log

- 2026-04-20 created by analyzer (`patch_jump` in `bytecode.rs` line 196
  contains a `panic!` in production compiler code; CLAUDE.md requires all
  compiler error paths to return typed errors, not panics)
