# Resilient FFI Design

**Status**: Draft — design-only, no implementation yet
**Date**: 2026-04-19
**Author**: Eric Spencer
**Covers goalposts**: new (tentative G21 — Foreign Function Interface)

---

## 1. Motivation

Resilient today is closed. It can run pure programs, call its own stdlib, and
that's it. To be useful as anything beyond a teaching language it needs a way
to reach into the host's native world — specifically:

- **CLI users** want to call OS and C-library functions (`libc`, `libm`,
  platform APIs) from Resilient source without recompiling the interpreter.
- **Embedded users** want to call vendor HAL code (ST HAL, nRF SDK, ESP-IDF)
  from inside a Resilient program running on an MCU, without dragging `dlopen`
  or an OS into the build.

Both needs reduce to the same language-level question: how does a Resilient
source program name, type, and invoke a non-Resilient function?

This spec answers that.

---

## 2. Goals and Non-Goals

### Goals

- **Single source-level syntax** for both `std` hosts and `no_std` embedded —
  the loader decides at startup which mechanism resolves the symbol.
- **Primitive-only v1** — `Int`, `Float`, `Bool`, `String` (UTF-8, borrowed),
  `Void`. Enough to call most of `libc` / `libm` / basic HAL calls.
- **Zero panics on the loader path.** Missing library, missing symbol, wrong
  arity, and type-mismatched argument are all clean typed errors.
- **Feature-gated.** A `resilient` build with `--no-default-features` OR
  a `resilient-runtime` build without `ffi-static` does not link `libloading`
  and does not pull in the registration table.
- **Contract-aware.** `requires` and `ensures` on an `extern fn` are checked
  at runtime; SMT treats FFI bodies as opaque (no verification can "see in").
- **Uniform across execution paths** — tree-walker interpreter, bytecode VM,
  and JIT all dispatch through the same loader API.

### Non-goals (v1)

- Struct / array / map marshalling across the boundary.
- Callbacks (Resilient fn passed as a C function pointer).
- Automatic C header parsing (`bindgen`-style).
- Multiple calling conventions (`stdcall`, `fastcall`). C `cdecl` only.
- Memory ownership transfer. All strings passed to foreign code are borrowed
  for the duration of the call; foreign code must not retain the pointer.
- Dynamic library unloading.
- Variadic foreign functions (e.g. `printf`).

Each non-goal is a named follow-up ticket.

---

## 3. Surface Syntax

### 3.1 Block form

```resilient
extern "libm.so.6" {
    fn sin(x: Float) -> Float;
    fn cos(x: Float) -> Float;
    fn pow(base: Float, exp: Float) -> Float;
}
```

- The string after `extern` is a **library descriptor**:
  - `"libm.so.6"` / `"libfoo.dylib"` / `"foo.dll"` — platform-specific path or
    SONAME; passed through to `libloading` verbatim on `std` hosts.
  - `"@static"` — sentinel telling the loader to look in the static
    registration table instead of calling `dlopen`. Required on `no_std`;
    optional on `std` as a way to force the static path.
- Each inner declaration is a standard Resilient fn signature minus the body,
  terminated by `;`. Parameter names are **mandatory** for documentation /
  error messages even though they're not used by the loader.
- Signatures use **Resilient** type names (`Int`, `Float`, `Bool`, `String`,
  `Void`) — the compiler lowers them to the C ABI at call time.

### 3.2 Symbol-name alias

When the Resilient name differs from the C symbol:

```resilient
extern "libm.so.6" {
    fn sine(x: Float) -> Float = "sin";
    fn cosine(x: Float) -> Float = "cos";
}
```

The `= "name"` clause overrides the symbol lookup; without it the Resilient
name is used verbatim.

### 3.3 Contracts on extern decls

```resilient
extern "libm.so.6" {
    fn sqrt(x: Float) -> Float
        requires(x >= 0.0)
        ensures(result >= 0.0);
}
```

- `requires` is checked on the **caller side** before the foreign call is
  made. Runtime only — SMT cannot verify a foreign body.
- `ensures` is checked on return. Foreign code that violates `ensures`
  produces the same contract-violation diagnostic as a Resilient fn would.
- `@trusted` above the block (or a single decl) turns `ensures` into an
  unchecked assumption that propagates into SMT at call sites. This is the
  FFI analogue of `unsafe` — buyer beware, it's how you integrate with
  libraries you can't verify but do trust.

### 3.4 Purity

- FFI decls are implicitly `@impure`. The purity checker rejects `@pure`
  on an `extern fn`.
- A `@pure` Resilient fn cannot call an FFI fn, even a trusted one. (Rationale:
  purity in Resilient is structural, not nominal. Breaking that for FFI would
  break inference in ways we don't want to debug.)

### 3.5 AST node

A new `Node::Extern` variant:

```rust
Node::Extern {
    library: String,           // "libm.so.6" or "@static"
    decls: Vec<ExternDecl>,    // each fn inside the block
    span: Span,
}

struct ExternDecl {
    resilient_name: String,
    c_name: String,            // == resilient_name unless `= "..."` given
    parameters: Vec<(String, String)>,  // (type, name)
    return_type: String,       // Resilient type name, "Void" for unit
    requires: Vec<Node>,
    ensures: Vec<Node>,
    trusted: bool,
    span: Span,
}
```

---

## 4. Type Mapping (v1)

| Resilient | C ABI                         | Notes                               |
| --------- | ----------------------------- | ----------------------------------- |
| `Int`     | `int64_t`                     | Wraps on overflow, matches VM.      |
| `Float`   | `double`                      |                                     |
| `Bool`    | `_Bool` (one byte)            |                                     |
| `String`  | `(const char*, size_t)`       | Two args on the C side, borrowed, UTF-8, no trailing NUL guaranteed. |
| `Void`    | `void`                        | Return type only.                   |

Anything else (`Array`, `Struct`, `Map`, `Set`, `Result`, `Bytes`, closures)
is a typecheck error on the FFI boundary in v1. Future tickets can add
`Bytes ↔ (uint8_t*, size_t)`, opaque handle types, etc.

---

## 5. Loader Architecture

### 5.1 `std` path (`--features ffi`, default on hosted)

New crate-internal module `resilient/src/ffi.rs`:

```
pub struct ForeignLoader {
    libs: HashMap<String, libloading::Library>,
    symbols: HashMap<(String, String), ForeignSymbol>,  // (lib, name) -> resolved
}

pub struct ForeignSymbol {
    ptr: *const (),                          // raw symbol
    signature: ForeignSignature,             // cached from the decl
}
```

- On program load (right after `imports::expand_uses`), the driver walks every
  `Node::Extern`, opens each distinct library once, and resolves every
  declared symbol eagerly. Any miss = clean `FfiError::{LibNotFound, SymbolNotFound}`.
- Resolved symbols are stored in a flat `HashMap` keyed by Resilient name so
  call sites look up in O(1).

### 5.2 `no_std` path (`resilient-runtime` with `ffi-static`)

`resilient-runtime` gains a module `ffi_static.rs`:

```rust
pub type ForeignFn = extern "C" fn();  // cast to real signature at call

pub struct StaticRegistry {
    entries: [Option<(&'static str, ForeignFn, ForeignSignature)>; N],
}

impl StaticRegistry {
    pub const fn new() -> Self { ... }
    pub fn register(
        &mut self,
        name: &'static str,
        ptr: ForeignFn,
        sig: ForeignSignature,
    ) -> Result<(), FfiError>;
    pub fn lookup(&self, name: &str) -> Option<ForeignSymbol>;
}
```

- No `HashMap` (heap-free default). Fixed-size array of entries; the
  embedding application picks `N` at compile time via a cargo feature
  (`ffi-static-64`, `ffi-static-256`). Default 64.
- `register` called by the embedder BEFORE `run(program)`. Registering a name
  twice is an error.
- Library descriptor `"@static"` dispatches to this registry; any other
  descriptor on `no_std` is a compile-time error (the `libloading` path
  isn't linked).

### 5.3 Shared signature layout

```rust
struct ForeignSignature {
    params: &'static [FfiType],
    ret: FfiType,
}

enum FfiType { Int, Float, Bool, Str, Void }
```

This lives in a shared module both the `std` loader and the `no_std`
registry depend on. Keeps the calling-convention glue in one place.

---

## 6. Call Path

### 6.1 Tree-walker interpreter

When the interpreter encounters a call whose resolved callee is a
`Value::Foreign`, it:

1. Pulls arg `Value`s off the top of the expression-eval stack.
2. Validates `args.len() == sig.params.len()` — type error otherwise.
3. Converts each `Value` to its C representation per the table in §4.
4. Transmutes the raw symbol pointer to a `extern "C" fn(...)` of the right
   shape — **one monomorphized trampoline per (arity, return-type)**, not per
   callsite. Up to 8 params in v1 = 9×5 = 45 trampolines. Generated by a
   `build.rs` macro or hand-rolled.
5. Calls the trampoline, catches the return value.
6. Converts the C return back to a `Value`.
7. Runs `ensures` checks (if any).

A new `Value` variant:

```rust
Value::Foreign {
    name: &'static str,
    ptr: *const (),
    sig: ForeignSignature,
    requires: Vec<Node>,
    ensures: Vec<Node>,
    trusted: bool,
}
```

### 6.2 Bytecode VM

The VM gains one new opcode:

```
OP_CALL_FOREIGN <symbol_index: u16> <arg_count: u8>
```

`symbol_index` indexes into a per-chunk foreign-symbol table populated by the
compiler. Dispatch is identical to the tree-walker's path (§6.1) but reads
args off the operand stack.

### 6.3 JIT (Cranelift)

The JIT lowers `OP_CALL_FOREIGN` to a direct `call_indirect` against the
resolved pointer. Argument and return marshalling is emitted inline as
Cranelift IR — no boxed `Value` round-trip, which is the whole point of
the JIT. Only calls with all-primitive signatures are JIT-lowered in v1;
anything with `String` falls back to the bytecode-VM path.

### 6.4 Loader wiring

All three paths share one resolved `Arc<ForeignLoader>` (or on `no_std`, a
`&'static StaticRegistry`) captured at program-load time. The tree-walker and
VM look up by name; the JIT captures the raw pointer at codegen time.

---

## 7. Error Model

Load-time errors (returned from `expand_uses`' FFI sibling pass):

- `FfiError::LibNotFound { library, underlying }`
- `FfiError::SymbolNotFound { library, symbol }`
- `FfiError::DuplicateRegistration { symbol }` (no_std)
- `FfiError::UnsupportedType { decl, type_name }`
- `FfiError::StaticOnlyOnStd { library }` — user asked for a dynamic
  library but built with `--no-default-features`.

Call-time errors:

- `RuntimeError::FfiArityMismatch { name, expected, got }`
- `RuntimeError::FfiTypeMismatch { name, param_index, expected, got }`
- `RuntimeError::Contract { kind: Pre | Post, fn: "name" }` (reuses the
  existing contract-violation path)

Inside the foreign function: **undefined**. C code that panics / aborts /
returns garbage is the embedder's problem. Document this prominently.

---

## 8. Feature Flags

### `resilient` crate

- `ffi` (**default on std, off on no_std builds if any**): enables
  `libloading` dep, `Node::Extern` parsing, `Value::Foreign`, and the call
  path in all three backends. Without it, `extern` blocks parse as a clean
  "FFI disabled in this build" typed error.

### `resilient-runtime` crate

- `ffi-static`: enables the static registration table. Zero-cost when off —
  no table, no opcode handler, no symbol storage.
- `ffi-static-64` / `ffi-static-256` / `ffi-static-1024`: pick table capacity.
  Mutually exclusive via `compile_error!` guard (same pattern as
  `alloc` / `static-only`).

---

## 9. Tree-walker-First Rollout (scope sequencing)

The implementation plan will decompose into at least these tickets:

1. **Parser** — recognize `extern "lib" { ... }` block and `= "c_name"` alias.
   AST node + tests.
2. **Typechecker** — reject non-primitive types in FFI signatures. Reject
   `@pure` on extern decls.
3. **Loader (std)** — `libloading` integration + eager resolution pass.
4. **Trampoline table** — generated `extern "C" fn` shims for all (arity,
   return) combinations through arity 8.
5. **Tree-walker call path** — `Value::Foreign`, dispatch, marshalling.
6. **Contract checks on FFI** — `requires` on entry, `ensures` on exit, shared
   with the Resilient-fn path.
7. **`@trusted` + SMT assumption** — pipe `trusted` through to the verifier.
8. **`resilient-runtime` ffi-static** — static registry + `register_foreign`
   API + tests + Cortex-M demo app that actually calls HAL.
9. **Bytecode VM** — `OP_CALL_FOREIGN` + tests.
10. **JIT** — Cranelift lowering + test that a tight loop calling
    `pow(x, 2.0)` beats the tree-walker by the JIT's usual margin.
11. **Docs** — SYNTAX.md entry, example program, Jekyll page.

Each ticket can ship independently and is runnable on its own. No ticket
depends on an un-landed follow-up.

---

## 10. Testing Strategy

- **Parser**: golden tests for `extern` block parsing, alias, contracts, error
  recovery on malformed blocks.
- **Typechecker**: reject-cases (array param, struct return, `@pure`).
- **Loader**: a tiny companion C file (`tests/ffi/lib_testhelper.c`) compiled
  into a `.so` / `.dylib` / `.dll` per-platform in `build.rs`. Covers
  successful load, missing library, missing symbol, valid call, type
  mismatch.
- **Tree-walker**: end-to-end example calling `libm::sqrt` on macOS/Linux
  hosts; skipped on Windows until a `libm` equivalent is selected.
- **`no_std` registry**: unit tests in `resilient-runtime` plus a new
  integration test in `resilient-runtime-cortex-m-demo` that calls a stub
  HAL function.
- **Contracts**: pre- and post-condition violation tests on FFI decls.
- **VM + JIT**: regression tests confirming identical behaviour across all
  three execution paths.

---

## 11. Open Questions (deferred)

These are flagged now so they don't sneak back in as v1 scope:

- Struct marshalling: repr? alignment? ownership?
- Callbacks: do they capture environment? How does a closure lower to a C
  fn pointer?
- Variadic calls: do we expose `printf`? (Probably not.)
- Multi-thread foreign calls: do we need a per-symbol lock? (Default: no —
  the embedder owns thread safety of the native code.)
- Library-version pinning: should the descriptor allow SemVer constraints?
- Hot-reload: `libloading` supports it, but the VM doesn't.

Each becomes a follow-up ticket when someone asks for it.

---

## 12. Success Criteria

FFI v1 ships when:

- A Resilient program can `extern "libm.so.6" { fn sqrt(x: Float) -> Float; }`
  and call it from all three execution paths, on both Linux and macOS.
- A Cortex-M demo app registers a stub HAL fn statically, a Resilient program
  on the MCU calls it, the call returns, the program continues.
- `cargo test` is green on host, `cargo test --features ffi` is green,
  `cargo test --features ffi-static` in `resilient-runtime` is green.
- Cross-compile for `thumbv7em-none-eabihf` still passes the size gate with
  `ffi-static` on.
- A contract violation on an FFI call surfaces as a clean diagnostic with
  `file:line:col`, matching the existing Resilient-fn contract-violation UX.
