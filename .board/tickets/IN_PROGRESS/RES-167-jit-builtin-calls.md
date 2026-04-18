---
id: RES-167
title: JIT: call builtins through indirect pointer (RES-072 Phase N)
state: OPEN
priority: P2
goalpost: G15
created: 2026-04-17
owner: executor
---

## Summary
The JIT today can't call `println`, `abs`, or any builtin — it
bails with Unsupported. Resolve the builtin name to an absolute
function-pointer at JIT-init and emit `call_indirect`. This is the
same mechanism RES-166 uses for runtime shims, generalized to the
full builtin table.

## Acceptance criteria
- `LowerCtx` gains a `builtin_addrs: HashMap<&'static str, usize>`
  populated from the existing builtin registry at JIT-init time.
- `Node::Call` with a builtin name on the callee:
  - Look up the address; if missing, Unsupported (no builtin by
    that name).
  - Cranelift sig mirrors the builtin's type.
  - `bcx.ins().iconst(I64, addr as i64)` + `call_indirect`.
- Mixed-type builtins (e.g. `pow` from RES-055 returns Int or
  Float depending on arg types) dispatch via the typechecker's
  recorded overload: we already monomorphize by types post-RES-124,
  so the JIT sees exactly one variant per call site.
- Unit tests: `println(1)`, `abs(-5)`, `pow(2, 10)` all JIT-compile
  and return correct values.
- Commit message: `RES-167: JIT builtin calls via indirect pointer (Phase N)`.

## Notes
- `println` has side-effecting IO — `bcx.ins().call_indirect(...)`
  is fine; Cranelift does not reorder across calls.
- Validate at JIT-init: if the typechecker ever resolved a call
  to a builtin by a name not in `builtin_addrs`, that's a bug in
  the registration wiring, not a user-facing error. Assert.

## Log
- 2026-04-17 created by manager
