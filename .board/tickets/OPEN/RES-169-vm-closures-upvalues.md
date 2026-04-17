---
id: RES-169
title: VM: closure Op + upvalue handling
state: OPEN
priority: P2
goalpost: G15
created: 2026-04-17
owner: executor
---

## Summary
The bytecode VM shipped function-call support in RES-081 but not
closures. Add Op::MakeClosure and upvalue arrays so the VM can run
every program the interpreter can. Matches the JIT's capture-by-
value approach (RES-164) to keep semantics identical.

## Acceptance criteria
- New opcode `MakeClosure { fn_idx: u16, upvalue_count: u8 }`
  followed by upvalue slot IDs inline in the bytecode.
- Runtime closure representation: `struct Closure { fn_ref:
  FnRef, upvalues: Box<[Value]> }`. VM values gain a `Closure`
  variant.
- Capture semantics: by value (copy at MakeClosure). Matches the
  interpreter / JIT for consistency.
- `Op::Call` on a closure pulls upvalues into the callee frame as
  a second local block, accessible via `Op::LoadUpvalue { idx }`.
- Unit tests: counter-maker closure, adder closure, nested
  closures.
- Commit message: `RES-169: VM closures via MakeClosure + upvalues`.

## Notes
- Upvalues are distinct from locals in the slot numbering — use
  `Op::LoadLocal` for params/locals, `Op::LoadUpvalue` for
  captures.
- The benchmark `closure_sum` should land alongside this ticket
  so we can contrast VM vs tree-walker.

## Log
- 2026-04-17 created by manager
