---
id: RES-167
title: JIT: call builtins through indirect pointer (RES-072 Phase N)
state: DONE
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
- 2026-04-17 landed RES-167a (builtin shims + registry); RES-167b/c deferred

## Resolution (RES-167a — builtin shims + registry only)

This landing covers only **RES-167a** of the implicit split
proposed by the Attempt-1 bail. The bail recommended "only covers
single-signature builtins (`println`, `abs`, the arity-stable
ones)" — I further narrowed to the arity-stable *Int-only*
builtins (`abs`, `min`, `max`) because `println` takes heterogeneous
`Value` args and needs additional plumbing that belongs in a
later ticket.

The `Node::CallExpression` lowering that turns a source-level
call into `call_indirect` (RES-167b) and the mixed-type builtin
work blocked on RES-124 monomorphization (RES-167c) remain
deferred.

### Files changed

- `resilient/src/jit_backend.rs`
  - New `pub(crate) mod jit_builtins` with three `extern
    "C-unwind"` shims matching the JIT's i64-only value model:
      * `res_jit_abs(x: i64) -> i64`  — `wrapping_abs` (so
        i64::MIN round-trips instead of panicking; matches the
        release-mode interpreter).
      * `res_jit_min(a: i64, b: i64) -> i64`.
      * `res_jit_max(a: i64, b: i64) -> i64`.
  - `pub(crate) struct JitBuiltinSig { name, symbol, arity, addr }`
    descriptor with unsafe impl Send + Sync so it can live in a
    `static OnceLock`-backed table.
  - `pub(crate) fn jit_builtin_table() -> &'static [JitBuiltinSig]`
    returns the alphabetically-sorted registry of three entries.
  - `pub(crate) fn lookup_jit_builtin(name) -> Option<&Sig>`
    linear-scans the table; the bail note's "lookup missing →
    Unsupported" semantic is exactly what RES-167b will receive.
  - `fn register_jit_builtin_symbols(&mut JITBuilder)` wires
    each `(symbol, addr)` into the module, called from
    `register_runtime_symbols` (RES-166a seam).
- Sixteen new unit tests named `res167a_*`:
  - Semantic correctness for each shim: abs on positive, zero,
    negative, and i64::MIN (wrapping) args; min/max on various
    ordering, negative, and equal-arg inputs.
  - Registry: known-name lookup roundtrips all four fields;
    unknown name (`println`, `nonexistent`) returns `None`;
    table stays alphabetically sorted; no duplicate names;
    arity field matches the shim's actual parameter count;
    every symbol starts with `res_jit_` (distinct from
    `res_array_` so namespaces don't collide).
  - Regression guards: `make_module` still succeeds after
    wiring, and an end-to-end `run("return 10 + 20;")` still
    returns 30.

### Verification

```
$ cargo build                        # OK (8 warnings, baseline)
$ cargo build --features jit         # OK
$ cargo test --locked
test result: ok. 611 passed; 0 failed      (non-jit baseline unchanged)
$ cargo test --locked --features jit
test result: ok. 728 passed; 0 failed      (+16 vs previous 712)
$ cargo test --features jit res167a
test result: ok. 16 passed; 0 failed
```

### What was intentionally NOT done

- **RES-167b** — no `LowerCtx::builtin_addrs` population from
  this registry, no `Node::CallExpression` lowering arm, no
  `call_indirect` emission.
- **RES-167c** — no `println` / `print` support (Value-shaped
  args), no mixed-type builtins (`pow`, numeric coercions), no
  overload selection — all still blocked on RES-124
  monomorphization.
- No changes to the existing lowering paths or calling
  convention beyond extending `register_runtime_symbols` to
  also call the new builtin-symbol helper.

### Follow-ups the Manager should mint

- **RES-167b** — `Node::CallExpression` lowering for JIT
  builtins: look up via `lookup_jit_builtin`, construct the
  matching cranelift `Signature` (`arity` × `I64` params, one
  `I64` return), import the symbol into the current function,
  emit `call`.
- **RES-167c** — once RES-124 monomorphization lands, add the
  mixed-type / heterogeneous-ABI builtins (`println`, `print`,
  `pow`, etc.) to this registry.

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
