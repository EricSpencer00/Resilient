# FFI v2 Design — VM + JIT + Arity Extension

**Date:** 2026-04-20  
**Status:** Approved → Implementation  
**Scope:** Extend FFI from interpreter-only (Phase 1) to all three backends

---

## Context

FFI v1 (already shipped) covers:
- Language syntax: `extern "libname" { fn foo(Int x) -> Float; }`
- Interpreter: `ForeignLoader` + `ffi_trampolines` (arity 0–2, primitives)
- `#[cfg(feature = "ffi")]` for dynamic loading via `libloading`

**Gaps this design closes:**
1. `compiler.rs` ignores `Node::Extern` — bytecode VM cannot call foreign fns
2. `vm.rs` has no `Op::CallForeign` opcode
3. `jit_backend.rs` has no foreign-symbol support
4. Trampolines only cover arity 0–2

---

## Architecture

```
extern "libm.so" { fn sin(Float x) -> Float; }

        │ parse (already done)
        ▼
Node::Extern { library, decls }

        │ interpreter (already done)          │ compiler (NEW)
        ▼                                     ▼
ForeignLoader::resolve_block()         Program::foreign_syms: Vec<ForeignSymbol>
        │                                     │
        ▼                                     ▼
call_foreign() via trampolines         Op::CallForeign(u16) → vm::run()
                                              │
                                             JIT (NEW)
                                              ▼
                                       Cranelift func_ref → direct native call
```

---

## Changes by file

### 1. `ffi_trampolines.rs` — extend arity 0–8

Add `dispatch_explicit` arms for:
- Arity 3: `(Int,Int,Int)→Int`, `(Float,Float,Float)→Float`, `(Int,Int,Int)→Void`
- Arity 4–8: `(Int…)→Int`, `(Int…)→Void` via variadic macro expansion
- Mixed types (Int+Float combos) for the most common C library patterns

No new types — just more arms in the `match (params, ret)`.

### 2. `bytecode.rs` — new opcode

```rust
/// Call a foreign (C ABI) function. The index references
/// `Program::foreign_syms[idx]`. Arity is determined by the symbol's
/// signature; the VM pops that many values, calls the trampoline,
/// pushes the result.
CallForeign(u16),
```

Add `CallForeign` to the `Op` enum and to the disassembler in `disasm.rs`.

### 3. `bytecode.rs` — extend `Program`

```rust
pub struct Program {
    pub main: Chunk,
    pub functions: Vec<Function>,
    /// FFI v2: resolved foreign symbols, indexed by Op::CallForeign(u16).
    #[cfg(feature = "ffi")]
    pub foreign_syms: Vec<std::sync::Arc<crate::ffi::ForeignSymbol>>,
}
```

`foreign_syms` is a `Vec` so `Op::CallForeign(u16)` indexes into it directly — O(1) dispatch with no HashMap at runtime.

### 4. `compiler.rs` — handle `Node::Extern`

In `compile_program()` (or equivalent top-level pass):

1. Collect all `Node::Extern` blocks.
2. Call `ForeignLoader::resolve_block(library, &decls)` for each — early-exit with a compile error on resolution failure.
3. For each resolved symbol, push into `program.foreign_syms`; record `name → index` in a `HashMap<String, u16>` local to the compiler.
4. When compiling a `Node::Call { callee: "sin", .. }` where `"sin"` is in the foreign map, emit `Op::CallForeign(idx)` instead of `Op::Call`.

### 5. `vm.rs` — handle `Op::CallForeign`

```rust
Op::CallForeign(idx) => {
    let sym = program.foreign_syms
        .get(idx as usize)
        .ok_or(VmError::FunctionOutOfBounds(idx))?;
    let arity = sym.sig.params.len();
    // Pop args (rightmost first)
    let mut args: Vec<Value> = (0..arity)
        .map(|_| stack.pop().ok_or(VmError::EmptyStack))
        .collect::<Result<_, _>>()?;
    args.reverse();
    let result = crate::ffi_trampolines::call_foreign(sym, &args)
        .map_err(|e| VmError::ForeignCallFailed(e))?;
    stack.push(result);
}
```

Add `VmError::ForeignCallFailed(String)` variant.

### 6. `jit_backend.rs` — Cranelift foreign calls

For each `Node::ExternDecl` the JIT encounters during compilation:

1. Build a Cranelift `Signature` from the `ForeignSignature` (Int→I64, Float→F64, etc.).
2. Call `module.declare_function(c_name, Linkage::Import, &sig)` to declare the import.
3. Get a `func_ref` via `module.declare_func_in_func(func_id, &mut func)`.
4. Emit `ins().call(func_ref, &[args…])` when the call site is reached.

This path calls the C symbol **directly** — no `Value` boxing, no trampoline overhead. The JIT path is the highest-performance path.

For the non-JIT `vm` path, use the trampoline approach above.

---

## Feature flag strategy

| Feature flag | What it enables |
|---|---|
| (none) | Syntax parses, `FfiDisabled` error at runtime |
| `--features ffi` | Dynamic loading (`libloading`) + trampolines + VM dispatch |
| `--features ffi,jit` | All of the above + Cranelift native calls |

The `no_std` embedded path is unaffected — `resilient-runtime` already has `@static` registry support in v1; the VM opcode path will be wired to it in a follow-up.

---

## Testing

1. **Unit**: extend `ffi_trampolines` tests for arity 3 arms.
2. **Integration** (`#[cfg(feature = "ffi")]`): 
   - Call `libm` `sin`/`cos`/`sqrt` via the **VM** backend (`--vm` flag).
   - Call `libm` via the **JIT** backend (`--jit` flag).
   - Verify same results as interpreter path.
3. **Golden example**: `resilient/examples/ffi_libm_demo.rs` + `.expected.txt`.
4. **Error paths**: arity mismatch, missing library, disabled feature — all clean errors.

---

## Tickets to open

- `RES-FFI-V2-TRAMPOLINES`: extend trampolines to arity 3–8
- `RES-FFI-V2-VM`: `Op::CallForeign` + compiler wiring
- `RES-FFI-V2-JIT`: Cranelift foreign call emission
- `RES-FFI-V2-EXAMPLE`: golden `ffi_libm_demo.rs`
