---
id: RES-167
title: JIT: call builtins through indirect pointer (RES-072 Phase N)
state: IN_PROGRESS
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
- 2026-04-17 claimed and bailed by executor (blocked + oversized)
- 2026-04-17 claimed by executor — landing RES-167a scope (builtin shims + registry only)
  after RES-166a unblocked the symbol-wiring half

## Attempt 1 failed

Two blockers.

1. **No JIT FFI wiring yet.** The JIT backend has no
   `JITBuilder::symbol(...)` registrations today (`grep -n "\.symbol("
   src/jit_backend.rs` → 0 hits). This ticket proposes a
   `builtin_addrs: HashMap<&'static str, usize>` populated from the
   builtin registry at JIT-init and consumed by a new
   `Node::CallExpression` lowering arm. Both pieces are brand-new
   Cranelift-facing code. Similar scope to RES-166 (also bailed).
2. **Depends on RES-124 monomorphization** for mixed-type builtins
   like `pow`, per the acceptance criteria: "we already monomorphize
   by types post-RES-124, so the JIT sees exactly one variant per
   call site." RES-124 is currently in OPEN with a `## Clarification
   needed` note (itself blocked on RES-120 and RES-122). Without
   monomorphization the JIT cannot pick a single address for a
   mixed-signature builtin.

## Clarification needed

Recommended sequencing: land RES-166a (the scaffolding half of
RES-166 — `mod runtime_shims` + `JITBuilder::symbol(...)` wiring)
first, then make RES-167 a follow-up that only covers single-
signature builtins (`println`, `abs`, the arity-stable ones). The
mixed-signature branch waits for RES-124's monomorphization pass.

No code changes landed — only the ticket state toggle and this
clarification note.
