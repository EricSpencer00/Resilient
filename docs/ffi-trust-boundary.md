---
layout: default
title: The FFI Trust Boundary
parent: Language Reference
nav_order: 11
---

# The FFI Trust Boundary
{: .no_toc }

<details open markdown="block">
  <summary>Table of contents</summary>
  {: .text-delta }
- TOC
{:toc}
</details>

Resilient's trust model says the verifier re-derives every safety claim
from the typed AST, and that anything it cannot derive is untrusted input
([STRUCTURAL_ENFORCEMENT.md](STRUCTURAL_ENFORCEMENT.md)).

An `extern` block is the sharpest place that claim gets tested. The
compiler cannot see inside a `.so`. It has no AST for the callee, no
control-flow graph, nothing to hand Z3. Everything on the other side of
the boundary is opaque by construction — not as an implementation gap
that a future ticket closes, but permanently.

So what does a contract on an `extern fn` actually mean?

## Three different things called "the contract"

```
extern "libm.so.6" {
    fn sqrt(x: Float) -> Float
        requires _0 >= 0.0
        ensures  result >= 0.0;
}
```

There are three distinct claims tangled together here, and they have
very different strengths.

### 1. `requires` — enforced, on our side

The precondition is checked before the call, on arguments Resilient
constructed and can see. If it fails, the call does not happen.

This is the strongest of the three. It is an ordinary runtime check on
ordinary Resilient values, and it holds regardless of what the library
does.

### 2. `ensures` without `@trusted` — checked, not proved

The postcondition is evaluated after the call returns, against the value
the library handed back. A violation raises a contract-violation error.

This is a **test**, not a proof. It tells you the library misbehaved on
*this* call with *these* arguments. It says nothing about the next call.
Z3 never sees it and never uses it.

That is still worth having — see the oracle pattern below — but do not
read `ensures` on an extern as the same object as `ensures` on a
Resilient function, where the verifier checks the clause against the
function body ([RES-4218](https://github.com/EricSpencer00/Resilient/pull/4223)).

### 3. `ensures` with `@trusted` — an axiom you are asserting

```
@trusted
fn fast_log(x: Float) -> Float requires _0 > 0.0 ensures result >= 0.0;
```

`@trusted` propagates the postcondition to Z3 **as an axiom**. Downstream
proofs may now rely on `fast_log` returning a non-negative number.

The clause is still evaluated at runtime, so a violation is reported —
but by then the proof that assumed it has already been built and shipped.
Nothing checks the axiom before Z3 consumes it.

This is the one place in the language where a human assertion enters the
proof without derivation. Treat every `@trusted` extern as an entry in
your safety case that needs its own evidence: a vendor qualification, a
DO-178C tool-qualification argument, a test campaign. The compiler is
recording your claim, not verifying it.

> If you are reaching for `@trusted` because a proof will not go through
> otherwise, you are converting a proof obligation into an assumption.
> That is sometimes the right engineering call. It is never a neutral one.

## Making an extern postcondition falsifiable

The `sqrt` example is unsatisfying precisely because `result >= 0.0` is
so weak. It would still hold if libm returned `0.0` for every input.

A postcondition earns its place when it can actually fail. The most
useful shape: **bind a library's fast path and its reference path, and
assert they agree.**

Many numerical, cryptographic, and signal-processing libraries ship both
— an optimised routine plus a slower obviously-correct one kept for
testing. Where that exists, the postcondition stops being an assertion
about the world and becomes a differential check.

```
extern "libhelper.so" {
    // Obviously correct, deliberately not clever.
    fn rt_isqrt_ref(n: Int) -> Int;

    // The fast path, checked against the reference on every call.
    fn rt_isqrt_fast(n: Int) -> Int
        requires _0 >= 0
        ensures  result == rt_isqrt_ref(_0);
}
```

Contract expressions may call other extern functions, which is what makes
this work. The clause is real code, evaluated after the call.

What this buys, precisely:

- Every call is checked against an independent implementation. Divergence
  is caught at the call that caused it, with the offending value in the
  diagnostic, rather than surfacing later as a wrong answer.
- The check is **falsifiable** — `resilient/examples/ffi_trusted_oracle.rz`
  binds a deliberately-wrong third variant and the clause fires.

What it does not buy:

- It is not a proof. Both implementations could be wrong in the same way.
- You pay for the reference path on every call. That is the entire cost
  model: a differential oracle is a debug- or qualification-build tool,
  not something to leave in a hot loop.
- It does not make `@trusted` safe. If you mark the fast path `@trusted`,
  Z3 assumes the agreement clause holds universally — which is a much
  stronger statement than "it held on the calls we made".

## Keeping the untrusted surface small

The boundary cannot be removed, so the useful question is how much rides
on it.

**Bind the narrowest thing that works.** Prefer `OpaquePtr` over teaching
the language a foreign struct's layout. A handle Resilient cannot
dereference is a smaller assumption than a layout that must stay in sync
with a header you do not control.

**Put the reasoning on your side of the boundary.** Wrap the extern in a
Resilient function, and put the contracts you actually want to prove on
*that*. The wrapper has a body the verifier can see:

```
fn safe_isqrt(Int n) -> Int
    requires n >= 0
    ensures  result >= 0
{
    if n == 0 { return 0; }
    return rt_isqrt_fast(n);
}
```

The `ensures` on `safe_isqrt` is now a clause the verifier attacks
against a body it can read, and the `n == 0` branch is discharged
outright.

Be clear-eyed about the rest of it, though. The other branch returns the
result of an opaque call, so Z3 has no fact about that value and reports:

```
warning[partial-proof]: Z3 returned Unknown for assertion — proof is incomplete
```

Wrapping does not conjure a proof out of a foreign call. What it buys is
narrower and still worth having: the obligation is now stated in a place
the verifier examines, the parts that do not depend on the extern are
discharged, and the residue is reported as an explicit `Unknown` instead
of never being asked. An unproven clause you can see beats an assumption
you cannot.

**Declare the signature from the header, not from memory.** Nothing
validates an `extern` declaration against the library. A wrong parameter
type is undefined behaviour that no Resilient-side check can catch — the
trampoline transmutes to whatever you declared. This is the load-bearing
assumption underneath every other one on this page.

**Record what you assumed.** Each `@trusted` extern is a line item in the
safety case. [EXPRESSIBLE_INVALID_STATES.md](EXPRESSIBLE_INVALID_STATES.md)
is the repository-level version of the same discipline.

## Caller-owned output buffers

When a C function fills storage supplied by its caller, use an explicit
buffer rather than an ordinary array. Arrays have value semantics: passing
one to a function gives that function a value copy, so there is no storage
for a C pointer to update. Buffers have reference semantics and keep their
typed allocation when cloned:

```resilient
let samples = buffer_float(256);
buffer_set(samples, 0, 1.5);
let alias = samples;
buffer_set(alias, 1, 2.5);
let snapshot = buffer_to_array(samples);
```

`buffer_int` and `buffer_float` allocate zero-filled storage. `buffer_len`
reports its element count, `buffer_get` and `buffer_set` enforce the buffer's
element type and bounds, and `buffer_to_array` makes an ordinary snapshot.
`Buffer<Int>` and `Buffer<Float>` can now be passed to an `extern` function as
caller-owned mutable `int64_t*` and `double*` storage. The borrow guard lasts
through the complete call, while the allocation remains owned by Resilient.

## A real-world binding: HST-core

`resilient/examples/ffi_hstcore.rz` is the worked binding for the optional
HST-core embedded ABI. It keeps the foreign handle opaque, uses `CStr` for the
artifact path and version string, passes sparse indices and values as copied
input arrays, and passes dense output through `Buffer<Float>`.

The example intentionally binds `hst_apply_delta` and `hst_recompute_full`
together. The first is the optimized sparse update; the second recomputes the
same held input state from scratch. `outputs_agree` compares their dense
buffers and places an `ensures result == true` postcondition on that comparison,
so a divergence becomes a runtime contract violation rather than a silent
benchmark result. This is differential evidence, not a proof that either
implementation is correct.

Two lifetime details are load-bearing:

- `hst_close` owns the opaque context lifetime; do not call any other binding
  with the handle after closing it.
- `hst_state` returns a borrowed pointer invalidated by later mutating calls,
  so the example deliberately uses the caller-owned output forms instead of
  exposing that pointer to Resilient code.

The optional example is marked `.interactive`, does not vendor a library or
operator artifact, and is skipped by the golden suite when the dependency is
absent. The exact ABI source is HST Studio's public `hstcore.h` and
`hstcore-abi/abi.json`; keep declarations synchronized with those files.

## Summary

| Construct | Enforced by | Strength |
|---|---|---|
| `requires` on an extern | Runtime check before the call | Holds — checked on values we own |
| `ensures` on an extern | Runtime check after the call | Per-call evidence only |
| `ensures` + differential oracle | Runtime check against a reference impl | Per-call, but genuinely falsifiable |
| `@trusted` + `ensures` | Nothing — asserted into Z3 | An assumption; needs external evidence |
| `ensures` on a Resilient fn | Z3, against the function body | Proof |

## See also

- [Foreign Function Interface](ffi.md) — types, trampolines, marshalling
- [STRUCTURAL_ENFORCEMENT.md](STRUCTURAL_ENFORCEMENT.md) — what is structural, what is external
- [VERIFICATION_LIMITS.md](VERIFICATION_LIMITS.md) — where verification's guarantees end
- `resilient/examples/ffi_trusted_oracle.rz` — the runnable oracle example
