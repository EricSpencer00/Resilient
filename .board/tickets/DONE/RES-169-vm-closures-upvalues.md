---
id: RES-169
title: VM: closure Op + upvalue handling
state: DONE
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
- 2026-04-17 landed RES-169a (opcodes + Value::Closure skeleton); RES-169b/c/d deferred

## Resolution (RES-169a — skeleton opcodes + Value::Closure)

This landing covers the remaining **RES-169a** scope. The
`free_vars(&Node) -> BTreeSet<String>` helper asked for in the
bail's clarification already shipped in **RES-164a** earlier this
session (`resilient/src/free_vars.rs`). This iteration completes
the 169a skeleton piece: `Op::MakeClosure`, `Op::LoadUpvalue`,
`Value::Closure`, plus matching dispatch / Debug / Display /
disasm arms.

Emission of the new opcodes from the bytecode compiler (RES-169b),
VM dispatch that actually implements the semantics (RES-169c),
and the end-to-end tests + benchmark (RES-169d) remain deferred.

### Files changed

- `resilient/src/bytecode.rs`
  - New `Op::MakeClosure { fn_idx: u16, upvalue_count: u8 }` and
    `Op::LoadUpvalue(u16)` variants with doc comments explaining
    the intended RES-169b/c semantics.
- `resilient/src/vm.rs`
  - New `VmError::Unsupported(&'static str)` variant + Display
    arm ("vm: unsupported opcode: {}").
  - New dispatch arms for both opcodes return `Unsupported(...)`
    so a misfired emission surfaces as a clean at-line error
    instead of panicking.
- `resilient/src/disasm.rs`
  - Disasm arms for both opcodes matching the printed format
    (`MakeClosure fn_idx upvalue_count  ; -> name` and
    `LoadUpvalue idx`).
- `resilient/src/main.rs`
  - New `Value::Closure { fn_idx: u16, upvalues: Box<[Value]> }`
    variant with `#[allow(dead_code)]` and doc comment. Added
    arms in `Debug` / `Display` impls so every exhaustive match
    stays complete (printing "Closure(fn=N, K upvalues)" /
    "<closure>" respectively).

### Tests (13 new, all `res169a_*`)

bytecode module:
- `res169a_make_closure_constructs_with_payload` — both operands round-trip.
- `res169a_load_upvalue_constructs_with_payload` — idx round-trips.
- `res169a_closure_ops_are_copy` — `Op: Copy` preserved across the new variants.
- `res169a_closure_ops_have_same_op_size_envelope` — regression guard on `sizeof(Op)`.
- `res169a_emit_make_closure_roundtrips_through_chunk` — emit/line_info pipeline.

vm module:
- `res169a_make_closure_dispatch_returns_unsupported` — clean error path.
- `res169a_load_upvalue_dispatch_returns_unsupported` — clean error path.
- `res169a_unsupported_error_display_is_descriptive` — Display contract.
- `res169a_closure_value_variant_constructs` — build + stringify a hand-built `Value::Closure`.
- `res169a_existing_vm_path_still_returns_correct_result` — 10 + 32 == 42 regression guard.

disasm module:
- `res169a_make_closure_renders_with_operands` — basic format.
- `res169a_load_upvalue_renders_with_idx` — basic format.
- `res169a_make_closure_with_named_fn_renders_pointer` — resolves `fn_idx` to fn name.

### Verification

```
$ cargo build                                   # OK (8 warnings, baseline)
$ cargo build --features z3                     # OK
$ cargo build --features lsp,logos-lexer,infer  # OK
$ cargo build --features jit                    # OK
$ cargo test --locked
test result: ok. 624 passed; 0 failed      (+13 vs 611)
$ cargo test --locked --features jit
test result: ok. 741 passed; 0 failed      (+13 vs 728)
$ cargo test res169a
test result: ok. 13 passed; 0 failed
```

### What was intentionally NOT done

- **RES-169b** — no bytecode-compiler emission of `Op::MakeClosure`
  for `Node::FunctionLiteral`, no upvalue slot discovery via the
  (now-available) `free_vars` helper.
- **RES-169c** — no real VM dispatch for `Op::MakeClosure` /
  `Op::LoadUpvalue`; no `CallFrame` extension to carry upvalues.
- **RES-169d** — no end-to-end closure tests (counter-maker,
  adder, nested), no `closure_sum` benchmark.
- No changes to the interpreter's existing `Value::Function`
  closure semantics.

### Follow-ups the Manager should mint

- **RES-169b** — bytecode compiler: for each `Node::FunctionLiteral`,
  call `free_vars`, emit `LoadLocal(src)` per capture, then
  `Op::MakeClosure { fn_idx, upvalue_count }`. Keep the upvalue
  ordering deterministic (`free_vars` already returns a
  `BTreeSet<String>`).
- **RES-169c** — VM: materialize a `Value::Closure` on
  `Op::MakeClosure`; extend `CallFrame` with an upvalue slab;
  implement `Op::LoadUpvalue`; make `Op::Call` accept a closure
  on the operand stack.
- **RES-169d** — the three end-to-end tests named in the ticket
  plus the `closure_sum` benchmark.

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
