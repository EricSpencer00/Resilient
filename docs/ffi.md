---
layout: default
title: Foreign Function Interface
nav_order: 15
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

Run with: `cargo run --features ffi -- examples/ffi_libm.rs`

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

| Resilient | C ABI     |
|-----------|-----------|
| `Int`     | `int64_t` |
| `Float`   | `double`  |
| `Bool`    | `bool`    |
| `String`  | not yet supported |
| `Void`    | `void`    |

At most 8 parameters per extern function.

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
