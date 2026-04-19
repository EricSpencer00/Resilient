---
id: RES-169
title: VM: closure Op + upvalue handling
state: IN_PROGRESS
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
- 2026-04-17 claimed and bailed by executor (oversized VM extension)
- 2026-04-17 claimed by executor — landing RES-169a scope (skeleton opcodes + Value::Closure)
  after RES-164a landed the free_vars walker

## Attempt 1 failed

Four distinct pieces bundled into one ticket:

1. **New opcodes** `Op::MakeClosure` + `Op::LoadUpvalue` (plus
   inline upvalue slot IDs) in `src/bytecode.rs`.
2. **New runtime closure value** — `Value::Closure` variant + arms
   in every Value match across `main.rs` / `vm.rs` / `compiler.rs`.
   Today `grep "Closure" src/vm.rs src/compiler.rs` returns 0
   hits.
3. **Bytecode-compiler upvalue pass** — free-var analysis for each
   `Node::FunctionLiteral`, emitting `MakeClosure` with the slot
   IDs. The interpreter has free-var enumeration inside
   `apply_function`, but it's not a reusable walker (same gap
   noted in RES-164's bail).
4. **VM dispatch** for both opcodes + `CallFrame` extension to
   carry upvalues as a parallel local block.

Plus three end-to-end tests and a `closure_sum` benchmark.

## Clarification needed

Manager, please split:

- RES-169a: extract `free_vars(&Node) -> BTreeSet<String>` helper
  (sharable with RES-164) and add `Value::Closure`,
  `Op::MakeClosure`, `Op::LoadUpvalue` as skeleton / unused
  variants.
- RES-169b: bytecode compiler emits `MakeClosure` for every
  `FunctionLiteral` that has captures, backed by 169a's walker.
- RES-169c: VM execution of the two new opcodes + CallFrame upvalue
  slot.
- RES-169d: the three end-to-end tests + `closure_sum` benchmark.

No code changes landed — only the ticket state toggle and this
clarification note.
