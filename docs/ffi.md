---
layout: default
title: Foreign Function Interface
parent: Language Reference
nav_order: 2
---

# Foreign Function Interface
{: .no_toc }

<details open markdown="block">
  <summary>Table of contents</summary>
  {: .text-delta }
- TOC
{:toc}
</details>

Resilient programs call into C libraries through `extern` blocks.

## Quick start

```
extern "libm.so.6" {
    fn sqrt(x: Float) -> Float requires _0 >= 0.0 ensures result >= 0.0;
}

fn main() {
    println(sqrt(16.0));   // prints: 4
    println(sqrt(2.0));    // prints: 1.4142135623730951
}
main();
```

Run with: `rz resilient/examples/ffi_libm.rz` (binary built with `--features ffi`).

## Extern block syntax

```
extern "LIBRARY_PATH" {
    fn NAME(PARAM: TYPE, ...) -> RETURN_TYPE [contracts];
}
```

`LIBRARY_PATH` is passed verbatim to the OS dynamic linker:

| Platform | libm path |
|----------|-----------|
| Linux    | `libm.so.6` |
| macOS    | `libm.dylib` |

## Supported types (v1)

Only primitive types are supported in FFI Phase 1:

| Resilient   | C ABI                                          |
|-------------|------------------------------------------------|
| `Int`       | `int64_t`                                      |
| `Int32`     | `int` / `int32_t` (RES-4226)                   |
| `Float`     | `double`                                       |
| `Bool`      | `bool`                                         |
| `String`    | variadic `printf`-style format strings; fixed-arity string ABI arms remain limited to implemented trampoline shapes |
| `Void`      | `void`                                         |
| `OpaquePtr` | `void*` (opaque)                               |
| `Array<Int>`   | `const int64_t*` (RES-4225; in-parameter only) |
| `Array<Float>` | `const double*` (RES-4225; in-parameter only)  |
| `CStr`      | `const char*`, NUL-terminated (RES-4226)       |
| `Callback`  | C function pointer (recognised in declarations; calls unsupported in Phase 1) |

At most 8 parameters per extern function.

String FFI is intentionally narrower than ordinary language string
support. The shipped trampoline table covers the documented
`fn c_printf(fmt: String, ...) -> Int` shape for `printf`-style
variadic calls; arbitrary C string ownership and fixed-arity `char*`
signatures need an explicitly documented trampoline arm before use.

### `OpaquePtr` — opaque C handles

An `OpaquePtr` is a `void*` the language can **receive, store, and
pass back** but never dereference. Use it to model C "handle" types
like `FILE*`, `sqlite3*`, or an allocator-owned struct pointer:

```
extern "libfoo.so" {
    fn alloc_point() -> OpaquePtr;
    fn free_point(p: OpaquePtr) -> Void;
    fn get_x(p: OpaquePtr) -> Int;
}

let p = alloc_point();
let x = get_x(p);
free_point(p);
```

Semantics:

- Resilient code cannot dereference, compare, or inspect the
  pointer — only pass it along to another FFI call.
- Lifetime is the C library's responsibility. If you leak a
  handle or use it after the C side freed it, that is a **use
  after free** on the C side; the language provides no safety net.
- Pass-through is zero-copy: the trampoline ferries the raw
  address across the ABI unchanged.

Trampoline coverage for pointer-bearing signatures is no longer
enumerated per shape. RES-4225 added an arity-only dispatch path: on
every ABI Resilient targets (SystemV x86-64, Windows x64, AArch64
AAPCS), `Int`, `OpaquePtr`, and `Array<T>` are all a single 64-bit
general-purpose argument register, assigned in declaration order. A
signature built only from those is therefore fully described by how
many registers it uses.

So **any** combination of `Int`, `Int32`, `CStr`, `OpaquePtr`, and
`Array<T>` parameters up to arity 8 works, returning `Int`, `Int32`,
`CStr`, `Float`, `Bool`, `OpaquePtr`, or `Void`:

```
extern "libfoo.so" {
    fn foo(ctx: OpaquePtr, name: CStr, idx: Array<Int>, n: Int32) -> Int32;
}
```

Adding `Int32` and `CStr` in RES-4226 required zero new dispatch arms —
both are one register, so they were already covered by arity.

Signatures with a `Float`, `Bool`, `String`, or struct **parameter**
still go through the explicit type-tuple table in `dispatch_explicit`,
which is extended by adding an arm.

### `Int32` — C's `int`

`Int` means `int64_t`. C's `int` is 32 bits, and on the **return** path
that difference is not cosmetic:

```
extern "libfoo.so" {
    fn foo_last_error() -> Int32;    // correct
    // fn foo_last_error() -> Int;   // WRONG: returns 4294967293 for -3
}
```

A C function returning `int` writes only `eax` / `w0`. The upper 32 bits
of the return register are not defined by the ABI, so reading the full
64 bits turns a returned `-3` into `4294967293`. Any C API that signals
errors with negative `int` codes must be declared `Int32`.

As a parameter, `Int32` range-checks the Resilient `Int` and **refuses**
values outside `-2147483648..=2147483647` rather than wrapping — a
silent truncation would hand C a different number than you wrote.

`Int` is unchanged and still means `int64_t`; this is additive.

### `CStr` — NUL-terminated `const char*`

```
extern "libfoo.so" {
    fn foo_open(path: CStr) -> OpaquePtr;
    fn foo_version() -> CStr;
}
```

Distinct from `String`, which marshals to a `(ptr, len)` pair and is only
reachable through the `printf`-style variadic path.

- **As a parameter**, the Resilient string is copied into a
  NUL-terminated buffer that lives for the duration of the call.
  Resilient strings may contain interior NUL bytes and C strings cannot,
  so a string containing one is **rejected**, not truncated at the first
  NUL.
- **As a return type**, the pointer is treated as **borrowed and
  library-owned**: the bytes are copied into a Resilient `String` and
  never freed. That matches `strerror()`, version strings, and any
  pointer into a static or library-managed buffer. If the library
  actually expected the caller to `free()` the result, this leaks — it
  does not corrupt, but it is an assumption the compiler cannot check.
  See [The FFI Trust Boundary](ffi-trust-boundary.md).
- A **null** return is a clean runtime error. If null is a valid result
  for your function, declare the return as `OpaquePtr` instead.
- A returned string that is not valid UTF-8 is a clean runtime error
  rather than a lossy conversion.

### `Array<T>` — buffer parameters

```
extern "libsum.so" {
    fn rt_sum_i64(xs: Array<Int>, n: Int) -> Int;
    fn rt_sum_f64(xs: Array<Float>, n: Int) -> Float;
}

println(rt_sum_i64([1, 2, 3, 4], 4));
```

`Array<Int>` lowers to `const int64_t*`, `Array<Float>` to
`const double*`. Element types other than `Int` and `Float` — including
nested arrays and `Array<String>` — have no contiguous C layout and are
rejected at compile time with a diagnostic that names the element type.

**The length is not passed implicitly.** C's convention is a separate
count parameter, and inventing a hidden one would make the Resilient
declaration disagree with the header it binds. Declare the count
yourself, as `n` above.

Semantics:

- **The buffer is a copy.** `Value::Array` is a vector of tagged enums,
  not a contiguous run of machine words, so there is no pointer into it
  to hand C. Each call allocates a `Vec<i64>` / `Vec<f64>`, copies the
  elements in, and frees it when the call returns. Passing a large array
  in a hot loop costs an O(n) copy per call.
- **The buffer is read-only from C's perspective and is not copied
  back.** Anything the callee writes through the pointer is discarded.
  Caller-owned output buffers are a separate type, tracked in RES-4226.
- **Elements must all match the declared element type.** No Int→Float
  widening: a silent coercion would hand C a bit pattern reinterpreted
  under a different type than the binding declares. Mismatches report
  the offending index.
- **Empty arrays pass a valid, non-null, aligned address with count 0.**
  Passing `NULL` instead would break callees that assert non-null before
  checking the count.
- **Arrays cannot be returned.** A C-allocated buffer has a lifetime
  Resilient cannot see, so `-> Array<Int>` is refused rather than
  handing back something that dangles.

See `resilient/examples/ffi_array_sum.rz`.

### `Callback` — declaration-only in Phase 1

Callback types are recognised in FFI declarations:

```
extern "libfoo.so" {
    fn register_handler(cb: Callback) -> Void;
}
```

Passing a Resilient function as `Callback` is not supported in Phase 1 and
returns a clean error:

```
FFI: extern fn `register_handler` uses a Callback parameter; callbacks
require the trampoline feature (planned for Phase 2)
```

Real trampoline support is planned for Phase 2 (bytecode VM).

## Contracts

Pre- and post-conditions work the same as on Resilient functions:

```
fn sqrt(x: Float) -> Float
    requires _0 >= 0.0
    ensures  result >= 0.0;
```

Arguments are bound **positionally** as `_0`, `_1`, … in `requires` clauses.
The return value is bound as `result` in `ensures` clauses.

Violations are caught at runtime before (or after) the C call, producing a
`contract violation` error.

## `@trusted` functions

> A contract on an `extern fn` is a different object from a contract on a
> Resilient function: the compiler cannot see the callee, so `ensures` is
> a per-call runtime check and `@trusted` turns it into an unverified Z3
> axiom. See [The FFI Trust Boundary](ffi-trust-boundary.md) for what
> each construct actually guarantees and how to keep the untrusted
> surface small.


```
@trusted
fn fast_log(x: Float) -> Float requires _0 > 0.0 ensures result >= 0.0;
```

`@trusted` propagates the `ensures` clause as an SMT axiom to the Z3
verifier. The `ensures` clause is still evaluated at runtime; a failure does
not abort the program. Instead, the clause is passed through and asserted as
an axiom for the Z3 verifier, which can reason about the foreign function's
postcondition without you needing to prove it inline.

## C symbol aliases

When the Resilient name differs from the C symbol, use `= "c_name"`:

```
extern "libc.so.6" {
    fn c_abs(x: Int) -> Int = "abs";
}
```

## `no_std` / embedded use

For `no_std` targets, the dynamic linker is unavailable. Use the
`resilient-runtime` crate's `StaticRegistry` instead:

```toml
[dependencies]
resilient-runtime = { features = ["ffi-static"] }
```

See the [no_std guide](no-std) for the full registration API.

## Design spec

See [the FFI design spec](superpowers/specs/2026-04-19-ffi-design) for the
full type model, contract semantics, and the roadmap for Phase 2 (bytecode
VM) and Phase 3 (Cranelift JIT).
