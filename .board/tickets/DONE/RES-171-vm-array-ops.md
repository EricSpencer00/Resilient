---
id: RES-171
title: VM: array literal + index load/store + push/pop opcodes
state: DONE
priority: P2
goalpost: G15
created: 2026-04-17
owner: executor
---

## Summary
Arrays are the last big value-kind the VM doesn't handle. With
RES-170 + this, the VM will cover all example programs that the
interpreter runs.

## Acceptance criteria
- Opcodes:
  - `MakeArray { len: u16 }` — pops len values, pushes Array.
  - `LoadIndex` — pops idx + arr, pushes arr[idx]. Bounds check
    inline with clean runtime error using per-statement line info
    (RES-092).
  - `StoreIndex` — pops v, idx, arr; writes back.
  - `ArrayPush` / `ArrayPop` / `ArraySlice` — call into runtime
    helper functions (same approach as interpreter / JIT shims).
- Compiler lowers `[a, b, c]` → 3 evals + MakeArray 3.
- `a[i] = v;` lowers to StoreIndex; support for nested
  (`a[i][j] = v`) produced by sequential LoadIndex + StoreIndex
  that matches RES-034 semantics.
- Unit tests: literal round-trip, indexing, push/pop/slice,
  nested assignment.
- Commit message: `RES-171: VM array ops`.

## Notes
- Watch the existing `bytecode.rs` opcode enum — keep the variant
  width reasonable. If the enum gets too wide, consider a
  `Op::ArrayOp(ArrayKind)` subvariant.
- Performance: the VM's array ops allocate and deallocate heap
  memory on every array manipulation. Acceptable; peephole pass
  (RES-172) can coalesce some patterns.

## Log
- 2026-04-17 created by manager
- 2026-04-17 claimed and bailed by executor (oversized; 6 opcodes +
  runtime helpers + compiler lowering + tests)
- 2026-04-17 claimed by executor — landing RES-171a scope (MakeArray + LoadIndex + StoreIndex)
- 2026-04-17 landed RES-171a (3 opcodes + dispatch + simple lowering); RES-171b/c deferred

## Resolution (RES-171a — MakeArray + LoadIndex + StoreIndex)

This landing covers the Attempt-1 split's "a" piece: three new
opcodes (`MakeArray`, `LoadIndex`, `StoreIndex`) with real VM
dispatch + bounds-check error paths, plus simple compiler
lowering for `Node::ArrayLiteral`, `Node::IndexExpression`, and
`Node::IndexAssignment` (where the target is a bare Identifier).

`ArrayPush` / `ArrayPop` / `ArraySlice` runtime helpers
(RES-171b) and nested `a[i][j] = v` lowering (RES-171c) remain
deferred.

### Files changed

- `resilient/src/bytecode.rs`
  - Three new `Op` variants:
      * `MakeArray { len: u16 }` — pop `len` values, wrap in
        `Value::Array`, push.
      * `LoadIndex` — pop idx + arr, push element (bounds/type
        checked).
      * `StoreIndex` — pop v + idx + arr, mutate, push modified
        array back (compiler follows with `StoreLocal`).
- `resilient/src/vm.rs`
  - New `VmError::ArrayIndexOutOfBounds { index: i64, len: usize }`
    with a `"vm: array index N out of bounds for length K"`
    Display.
  - Three new dispatch arms with inline bounds + type checks.
    `MakeArray` uses `stack.drain(split_at..)` for efficient
    contiguous pop. `StoreIndex` destructures the Array out,
    mutates, and pushes back.
- `resilient/src/compiler.rs`
  - `Node::ArrayLiteral` → emit each item, then `MakeArray`.
    Caps at u16::MAX elements.
  - `Node::IndexExpression` → emit target, emit index, `LoadIndex`.
    Nested reads (`a[i][j]`) fall out naturally through
    recursion.
  - `Node::IndexAssignment` → only supports bare-Identifier
    targets. Lowers to `LoadLocal(a), idx, v, StoreIndex,
    StoreLocal(a)`. Nested forms surface as
    `Unsupported("nested index assignment (RES-171c)")`.
  - Same pattern applied to both `compile_stmt` (main chunk) and
    `compile_stmt_in_fn` (function bodies).
- `resilient/src/disasm.rs`
  - Disasm arms for `MakeArray N`, `LoadIndex`, `StoreIndex`.

### Tests (16 new, all `res171a_*`)

Opcode-level (hand-built chunks):
- `make_array_from_three_constants` — basic shape round-trips.
- `make_array_empty_literal_returns_empty_array`.
- `make_array_stack_underflow_errors` — asks for more than is pushed.
- `load_index_reads_element` — `[10,20,30][1] == 20`.
- `load_index_out_of_bounds_errors` — OOB variant populated.
- `load_index_negative_index_errors` — negative idx → OOB.
- `load_index_non_int_errors` — Bool index → TypeMismatch.
- `store_index_writes_and_pushes_modified_array`.
- `store_index_oob_errors_without_modifying`.
- `store_index_display_is_descriptive` — Display text.

Compiler-integration (parse → compile → run):
- `compile_and_run_array_literal_index` — `a[1]` end-to-end.
- `compile_and_run_index_assign_then_read` — `a[1] = 99` round-trip.
- `compile_and_run_read_all_after_store` — store preserves siblings.
- `compile_rejects_nested_index_assignment` — clean error for `a[0][1] = v`.
- `empty_array_literal_compiles_and_runs`.
- `oob_read_from_compiled_program_surfaces_at_line` — runtime OOB
  comes through the AtLine wrapper.

### Verification

```
$ cargo build                                   # OK (8 warnings, baseline)
$ cargo build --features z3                     # OK
$ cargo build --features lsp,logos-lexer,infer  # OK
$ cargo build --features jit                    # OK
$ cargo test --locked
test result: ok. 651 passed; 0 failed      (+16 vs 635)
$ cargo test res171a
test result: ok. 16 passed; 0 failed
```

### What was intentionally NOT done

- **RES-171b** — no `ArrayPush` / `ArrayPop` / `ArraySlice`
  opcodes or runtime helpers. `push(a, x)` / `pop(a)` /
  `slice(a, i, j)` calls fall through the builtin-call path
  (which is itself `Unsupported` in the VM today).
- **RES-171c** — no nested `a[i][j] = v` lowering. The compiler
  emits a clean `Unsupported("nested index assignment (RES-171c)")`
  error rather than silently miscompiling.
- No changes to the interpreter or JIT paths — this is a pure
  VM extension.

### Follow-ups the Manager should mint

- **RES-171b** — add `ArrayPush`, `ArrayPop`, `ArraySlice`
  opcodes plus runtime helpers. Consider a shared `mod
  runtime_shims` pattern cross-cutting with RES-166a (JIT array
  shims already use the analogous seam).
- **RES-171c** — nested `a[i][j] = v` lowering matching RES-034
  semantics; requires a load-modify-store chain where each
  intermediate array is pulled out, mutated, and restored.

## Attempt 1 failed

Oversized: the ticket bundles four independently-sized pieces.

1. **Six new opcodes** (`MakeArray`, `LoadIndex`, `StoreIndex`,
   `ArrayPush`, `ArrayPop`, `ArraySlice`) in `src/bytecode.rs` +
   VM dispatch arms in `src/vm.rs`.
2. **Runtime helper functions** for push / pop / slice exposed to
   the VM — same scaffolding concept RES-166 introduces for the
   JIT's `mod runtime_shims`, which also doesn't exist yet.
3. **Compiler lowering** for `Node::ArrayLiteral`,
   `IndexExpression`, `IndexAssignment` (including the nested
   `a[i][j] = v` form). Today the VM compiler errors
   `Unsupported` on all of these.
4. **Per-op bounds-check error paths** carrying `line_info`
   (RES-092), plus the ticket's four end-to-end tests.

## Clarification needed

Manager, please split:

- RES-171a: `MakeArray` + `LoadIndex` + `StoreIndex` opcodes +
  dispatch + bounds-check error path. Compile `ArrayLiteral` +
  simple `IndexExpression` / `IndexAssignment`. Smallest self-
  contained slice.
- RES-171b: `ArrayPush` / `ArrayPop` / `ArraySlice` via runtime
  helpers. Consider hoisting the VM's runtime-shim scaffolding
  into its own shared ticket if the JIT side (RES-166) also wants
  it — both tickets propose parallel mod-level shims.
- RES-171c: nested `a[i][j] = v` lowering matching RES-034
  semantics.

No code changes landed — only the ticket state toggle and this
clarification note.
