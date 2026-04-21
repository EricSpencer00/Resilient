---
id: RES-285
title: "bytecode compiler (--vm): assert() and assume() produce CompileError::Unsupported"
state: DONE
Claimed-by: Claude Sonnet 4.6
priority: P3
goalpost: G15
created: 2026-04-20
owner: executor
---

## Summary

The bytecode compiler (`resilient/src/compiler.rs`) does not handle
`Node::Assert` or `Node::Assume` in either `compile_stmt` or
`compile_stmt_in_fn`. Both functions fall through to:

```rust
other => Err(CompileError::Unsupported(node_kind(other))),
```

`node_kind(Node::Assert { .. })` returns `"Assert"` and
`node_kind(Node::Assume { .. })` returns `"Assume"`.

As a result, running any program that contains an `assert(...)` or
`assume(...)` statement through the `--vm` flag produces:

```
VM compile error: Assert
```

or

```
VM compile error: Assume
```

This is a user-visible error with no actionable guidance, and it means
safety-critical runtime checks are silently absent when the bytecode path
is used.

## Affected code

`resilient/src/compiler.rs`:
- `fn compile_stmt` (line ~147) — `other => Err(CompileError::Unsupported(node_kind(other)))`
- `fn compile_stmt_in_fn` (line ~307) — same catch-all

Both functions need `Node::Assert { condition, message, .. }` and
`Node::Assume { condition, message, .. }` arms added.

## Acceptance criteria

Add lowering for both `assert` and `assume` in `compile_stmt` and
`compile_stmt_in_fn`. The semantics for the bytecode path should match
the tree-walker:

### assert(condition[, message])

Compile as a conditional runtime check:
1. Compile `condition` to the stack.
2. Emit `Op::JumpIfTrue(skip)` (skip the trap if condition holds).
3. Push the error message string (formatted: `"assertion failed: <source>"`
   or the user-supplied message) as a `Const`.
4. Emit `Op::RuntimeError` (or equivalent — use whatever opcode the VM
   uses for trapping with a message).
5. Patch `skip` to the next instruction.

If `RuntimeError` is not yet an opcode, add it as a new `Op` variant with
a string payload and a matching `VmError` variant.

### assume(condition[, message])

Same as `assert` — `assume` is a runtime assertion with the same
behavior; the verifier context threading is a separate concern (RES-235).

### Tests

Add a test in `resilient/tests/` or as a `#[test]` in `compiler.rs`:
- A program `assert(1 == 1);` compiled with `--vm` exits 0.
- A program `assert(1 == 0);` compiled with `--vm` exits non-zero with
  a message containing "assertion failed".
- A program `assume(true);` compiled with `--vm` exits 0.
- A program `assume(false);` compiled with `--vm` exits non-zero.

These are NEW tests — they do not modify existing tests.

- `cargo test` passes with 0 failures.
- `cargo clippy --all-targets -- -D warnings` clean.
- Commit: `RES-285: bytecode compiler — lower assert() and assume() to runtime checks`.

## Notes

- The tree-walker path (`Interpreter::eval`) correctly handles both
  `Node::Assert` and `Node::Assume` via `eval_assert` and `eval_assume`.
  This ticket only concerns the bytecode VM (`--vm`) path.
- The JIT path (`--jit`) may have the same gap; it is out of scope here
  but should be noted in the PR description for follow-up.
- Do NOT change the tree-walker semantics or any existing test.
- The `assume` feature was added in commit `6ada8e3` (PR #48); it shipped
  without bytecode compiler support because the bytecode path is
  experimental and was not in scope for that PR.

## Log

- 2026-04-20 created by analyzer (compiler.rs compile_stmt and
  compile_stmt_in_fn both fall through to Unsupported for Node::Assert and
  Node::Assume; running assert/assume with --vm produces confusing
  "VM compile error: Assert" with no guidance; assume() was just added in
  commit 6ada8e3 so this gap is newly relevant)
