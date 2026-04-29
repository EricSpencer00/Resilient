---
title: FFI Trampoline Security Audit
nav_order: 13
permalink: /security/ffi-audit
---

# FFI Trampoline Security Audit (RES-383)
{: .no_toc }

Focused security review of `resilient/src/ffi.rs` and
`resilient/src/ffi_trampolines.rs` covering the v1 FFI surface
introduced under G21 (shipped 2026-04-19).
{: .fs-6 .fw-300 }

<details open markdown="block">
  <summary>Table of contents</summary>
  {: .text-delta }
- TOC
{:toc}
</details>

---

## Audit scope

- Files: `resilient/src/ffi.rs` (loader / symbol resolution),
  `resilient/src/ffi_trampolines.rs` (call dispatch).
- All `unsafe` blocks and `unsafe impl` declarations.
- Variadic call boundary (max 17 args).
- libloading handle lifetime correctness.
- Type confusion between Resilient `Value` and C scalars.
- Use-after-free in library handle / symbol management.

## Audit results — summary

| Concern                                | Status                            |
| -------------------------------------- | --------------------------------- |
| Buffer overflow in trampoline dispatch | **No** — pattern-match dispatch.  |
| Type confusion at the C boundary       | **Mitigated** — typechecker gate. |
| libloading handle lifetime             | **Sound** — Library held in map.  |
| Variadic arg overflow                  | **Mitigated** — runtime bound.    |
| Send/Sync soundness                    | **Sound** — opaque pointer only.  |
| String-payload UB                      | **Sound** — `live_strs` lifetime. |

No CVE/CWE-class vulnerabilities identified at audit time. Two
design observations are recorded under "Recommendations" below.

## Per-block safety arguments

### `ffi.rs`

#### Block A — `unsafe impl Send for OpaquePtrHandle` (line ~188)

```rust
unsafe impl Send for OpaquePtrHandle {}
unsafe impl Sync for OpaquePtrHandle {}
```

**Safety argument.** `OpaquePtrHandle` wraps `*mut c_void` from a
foreign library. The pointer is opaque to Resilient — interpreter
code never dereferences it; the only operation is passing it
back to a subsequent FFI call. Sending the address across a
thread boundary is therefore equivalent to copying an integer.

**Verdict.** Sound. The C library is responsible for thread
safety of the pointee — that's outside Resilient's TCB.

#### Block B — `unsafe impl Send for ForeignSymbol` (line ~361)

```rust
unsafe impl Send for ForeignSymbol {}
unsafe impl Sync for ForeignSymbol {}
```

**Safety argument.** `ForeignSymbol` holds a raw fn pointer
extracted from a `libloading::Symbol`. The Symbol has been
released; only the bare pointer remains. The `Library` is held
inside `ForeignLoader::libs`, ensuring the code segment remains
mapped for as long as any symbol is reachable.

**Verdict.** Sound, contingent on **Recommendation 1** below.

#### Block C — `libloading::Library::new(library)` (line ~419)

```rust
let lib = unsafe { libloading::Library::new(library) }.map_err(...);
```

**Safety argument.** `Library::new` is unsafe because loading a
shared library executes `_init` / constructor code. The
trustworthiness of the library file is the user's responsibility
— Resilient intentionally does not enforce a sandbox here. The
v1 FFI README documents this risk (`docs/ffi.md`).

**Verdict.** Sound. The unsafety is a load-time exposure
inherited from `libloading`; Resilient surfaces it as an
explicit feature flag (`--features ffi`).

#### Block D — `lib.get(d.c_name.as_bytes())` (line ~435)

```rust
let raw: libloading::Symbol<*const ()> = unsafe { lib.get(...) };
```

**Safety argument.** The `Symbol` borrow is released by reading
the raw pointer out of it (`*sym` then drops the borrow). The
extracted `*const ()` is then transmuted to a typed fn pointer
at the call site (`ffi_trampolines.rs`). The `Library` itself
remains in `self.libs`, so the code segment is never unmapped
while the symbol is reachable.

**Verdict.** Sound.

### `ffi_trampolines.rs`

#### Block E — main trampoline `unsafe { ... }` (line ~409)

```rust
let out = unsafe {
    if variadic { /* call_variadic_ints! macros */ }
    else { /* match (params.as_slice(), ret) { ... } */ }
};
```

**Safety argument.** This is the largest unsafe block — the call
dispatch. Key invariants:

1. **Type validation.** The Resilient typechecker has already
   gated on `FfiType::{Int, Float, Bool, Void, ...}` (no
   structs / pointers in v1). The pattern match enumerates
   every (params, ret) combination explicitly; an unmatched
   combination falls through to a conservative
   `Err("unsupported FFI signature")` arm.
2. **Argument coercion.** `Value::Int(i64)` → `i64`, `Value::Float`
   → `f64`, `Value::Bool` → `bool` are direct moves. String
   args are routed via `live_strs` (see Block F below).
3. **No buffer overrun.** The arity is established by
   `args.len() == sym.sig.params.len()` before the match
   begins (line 47). The match itself enumerates fixed
   slot counts; there is no indexed write into a buffer.
4. **Transmute soundness.** `transmute::<*const (), extern "C"
   fn(...)>` reinterprets a pointer; on every supported
   platform the pointer width and ABI for fn types match raw
   pointer width.

**Verdict.** Sound under the typechecker gate. **Recommendation
2** records a hardening idea.

#### Block F — `live_strs` lifetime (line ~331, comment ~1028)

```rust
let mut live_strs: Vec<&[u8]> = Vec::with_capacity(args.len());
// ... `live_strs` and `strs` intentionally outlive the unsafe block
```

**Safety argument.** `live_strs` is constructed before the
unsafe block and dropped after it. C strings (`*const c_char`)
passed to the foreign call point into byte slices held by
`live_strs`. The Rust borrow checker enforces that the
references are valid for the lifetime of the FFI call.

**Verdict.** Sound. This is the standard Rust pattern for
crossing the C boundary with borrowed string data.

#### Block G — variadic transmute (line ~417)

See discussion under Block E. The variadic dispatch is the same
pattern but with `extern "C" fn(..., ...)`, gated on `args.len()
- params.len() ∈ {0..=16}` at line 101.

**Verdict.** Sound.

## Variadic arity boundary

The v1 trampoline supports up to **17 total arguments** for
variadic C functions (1 fixed + 16 variadic, see line 101–106).
Non-variadic calls support up to **arity 8** by enumeration in
the match dispatch.

### Can the boundary be abused?

**No.** The check at line 47 (`args.len() != params.len()`) and
line 93 (`args.len() < fixed`) and line 101 (`args.len() > 17`)
gate every code path before the unsafe block runs. An attacker
program supplying 18+ args is rejected at the call boundary
with a typed `Err`, before any pointer dispatch occurs.

The match statement itself has no fall-through that would
dispatch on an unverified arity — every arm matches an explicit
fixed pattern, and an unmatched signature returns
`Err("unsupported FFI signature")`.

## libloading handle lifetime

The `ForeignLoader` owns a `HashMap<String, Library>`. Every
`Symbol` extracted from a library copies the raw fn pointer out
of the borrow before returning, so the only lifetime that
matters is the `Library`'s. Library handles are inserted into
the map and never removed, ensuring the code segment is mapped
for the entire run of the program.

The `ForeignLoader` itself is owned by the `Interpreter`, which
is dropped at end of program. There is no public API to drop a
library mid-run.

**Verdict.** Sound. No use-after-free path exists in the v1
surface.

## Type confusion at the C boundary

The Resilient → C coercion is a closed mapping:

| Resilient   | C ABI                  |
| ----------- | ---------------------- |
| `Int`       | `i64`                  |
| `Float`     | `f64`                  |
| `Bool`      | `bool` (0 / 1)         |
| `String`    | `*const c_char` (UTF-8)|
| `Void` (ret)| `()`                   |

The typechecker rejects every other Resilient type at the
extern-decl boundary (RES-317 added struct support behind
`@repr(C)`, which is verified separately). A program that tries
to call a C function declared `int sqrt(double)` with an `Int`
argument is caught at the typechecker before reaching FFI.

**Verdict.** No type-confusion path identified within the v1
type whitelist.

## Threat model

- **Trusted.** The Rust toolchain, the libloading crate, the C
  library being called.
- **Untrusted input.** The Resilient program's `extern` block —
  the typechecker treats this as input and validates each
  declared type.
- **Out of scope.** The semantic correctness of the foreign
  function itself. If `extern "C" fn add_one(int) -> int`
  actually launches a missile, that's the user's problem.

## Recommendations (non-blocking)

### Recommendation 1 — symbol → library backref

Currently `ForeignSymbol` carries only the raw pointer; the
`Library` it came from is identified by name in the loader's
HashMap. If a future ticket adds dynamic library unloading
(`Library::close`), the symbol cache must be invalidated atomically
or a use-after-free becomes possible.

**Mitigation today:** `Library::close` is not exposed on
`ForeignLoader`. There is no public API path to UAF.

### Recommendation 2 — typechecker / trampoline drift gate

The trampoline match enumerates (param, ret) combinations
explicitly. If a future PR adds a new `FfiType` variant (e.g.
`Pointer` for opaque ptr return) but forgets to extend the
trampoline match, the program compiles but hits the
`unsupported signature` arm at runtime.

**Mitigation idea:** A `cargo test` that loops over every
FfiType pair and asserts the trampoline either dispatches or
returns the exact "unsupported" error. Implementation deferred.

## Fuzz corpus inputs

Two boundary-condition fuzz inputs to add to the FFI fuzz
harness (delivery deferred to a follow-up ticket — the harness
exists at `fuzz/fuzz_targets/`):

### Input 1 — variadic arity boundary

```c
extern "libc" {
    fn printf(fmt: String, ...) -> Int;
}
fn main() {
    // 17 total args (1 fixed + 16 variadic) — at the boundary.
    printf("%d %d %d %d %d %d %d %d %d %d %d %d %d %d %d %d\n",
           1, 2, 3, 4, 5, 6, 7, 8,
           9, 10, 11, 12, 13, 14, 15, 16);
}
```

**Expected:** runs to completion, prints the digits.

### Input 2 — variadic arity overflow

```c
extern "libc" {
    fn printf(fmt: String, ...) -> Int;
}
fn main() {
    // 18 total args — over the limit.
    printf("%d %d %d %d %d %d %d %d %d %d %d %d %d %d %d %d %d\n",
           1, 2, 3, 4, 5, 6, 7, 8,
           9, 10, 11, 12, 13, 14, 15, 16, 17);
}
```

**Expected:** runtime error
`"FFI: variadic call has 18 arguments; supported maximum is 17"`.

## Conclusion

The v1 FFI surface (RES-217 through RES-318) demonstrates a
defensive design: the typechecker gates type-related risk, the
trampoline gates arity-related risk, and the loader's
HashMap-of-Library construction handles lifetime risk. No CVE-
or CWE-class vulnerabilities were identified.

The v1 FFI is not a sandbox — a malicious foreign library can
do anything its process privileges allow. Resilient documents
this trust boundary explicitly in `docs/ffi.md`.

## Sign-off

- Auditor: agent (RES-383)
- Date: 2026-04-29
- Source revision: see git log on `docs/security/ffi-audit.md`.
- Follow-ups: Recommendation 1 (UAF gate when unload lands),
  Recommendation 2 (trampoline drift test). Both filed as
  hardening ideas; neither blocks the audit sign-off.
