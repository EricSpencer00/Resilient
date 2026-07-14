---
title: Failure Model
parent: Design Philosophy
nav_order: 6
permalink: /failure-model
---

# Resilient Failure Model

## Overview

Resilient gives a program four ways to talk about the ways it can fail:

1. **Checked failures** (`fails Variant, ...`) — a function declares, on its
   signature, the named failure variants it can raise. Callers must either
   propagate every variant on their own `fails` list, or discharge it with a
   `try { } catch Variant { }` handler.
2. **Contracts** (`requires` / `ensures` / `recovers_to:`) — precondition,
   postcondition, and (for functions inside a recovery path) a single-step
   "the invariant holds again" clause. Checked at runtime always; proved
   statically when built with `--features z3` and the verifier succeeds.
3. **Retry-based recovery** (`live { }` / `live invariant COND { }` blocks) —
   a block re-runs itself (optionally with backoff and a wall-clock budget)
   when an `assert()` inside it fails, instead of aborting the whole program.
4. **`Result<T, E>` / `Option<T>`** — ordinary value-level error handling for
   the failure modes that don't need a checked-failure declaration on the
   signature: parsing, I/O helper functions, anything expressed as
   `Ok`/`Err`/`Some`/`None` and matched explicitly.

These four are independent mechanisms that compose, not four rungs of one
escalating ladder — a function can declare `fails`, take `requires`, contain
a `live` block, and internally use `Result` for a helper call, all at once
(see `resilient/examples/assume_recovers_to.rz`, which combines `requires`,
`fails`, and `recovers_to:` on one signature).

This document intentionally does **not** re-describe the memory-safety or
region/borrow rules that prevent whole classes of failure at compile time —
see [MEMORY_MODEL.md](/memory-model) and `STABILITY.md` § Stable (region
annotations) for those.

---

## 1. Checked failures: `fails`

A function signature can declare the named failure variants it may raise:

```resilient
fn read_sensor(int addr)
    requires addr >= 0
    fails HardwareFault, Timeout
    recovers_to: addr >= 0;
{
    return addr;
}
```

**Propagation rule (RES-387):** a caller that invokes `read_sensor` must
either add `HardwareFault` and `Timeout` to its *own* `fails` list, or
discharge them explicitly:

```resilient
fn caller(int addr) {
    try {
        let v = read_sensor(addr);
        println(v);
    } catch Timeout {
        println(-1);
    }
}
```

**Discharge rule (RES-224):** each `catch Variant { ... }` arm subtracts that
variant from the propagation obligation for calls made inside the `try`
body. If every declared variant on every call inside the block is caught,
the surrounding function needs no `fails` clause of its own. A partially
handled call still requires the leftover variants on the caller's
signature — there's no way to silently swallow a variant.

**Runtime behavior (RES-775):** with the tree-walking interpreter, the
compiler can inject a declared checked failure deterministically (used for
testing recovery paths); the injected failure enters the callee's `try`
site as if the callee had actually raised that variant, so the matching
`catch` arm executes.

Checked failures are a distinct mechanism from `Result<T, E>` — they are
part of the function's *type signature* and checked by the typechecker
(propagation is a compile error, not a runtime possibility), whereas
`Result` is an ordinary value the caller chooses whether to inspect.

---

## 2. Contracts: `requires` / `ensures` / `recovers_to`

```resilient
fn max(int x, int y) -> int
    requires x >= 0
    requires y >= 0
    ensures return >= x
{
    if x > y { return x; }
    return y;
}
```

- **`requires`** — precondition on the caller. Violated calls fail at
  runtime with a contract diagnostic; with `--features z3`, the verifier
  attempts to prove every call site satisfies it statically.
- **`ensures`** — postcondition on the function's own return value.
- **`recovers_to: expr;`** — a postcondition specifically for the state a
  function (or the tail of a `live` block) leaves behind after handling a
  fault. It is a **single-transition** property — "this one step
  re-establishes the invariant" — not a temporal "eventually holds" claim.
  Multi-step recovery reasoning is a V2 verifier capability tracked under
  RES-396; don't read a V1 `recovers_to` success as a liveness guarantee.

**Verification tiers, honestly:**
- Default build: `requires`/`ensures`/`recovers_to` are runtime-checked
  assertions. A violation is a runtime error with a diagnostic; it is not
  proved absent.
- `--features z3`: the verifier attempts a static proof per function
  (`requires`/`ensures`, one-step `recovers_to`, snapshot cluster
  invariants). This surface is intentionally **state-local** — it does not
  reason about traces (liveness, fairness, multi-actor interleavings); see
  `STABILITY.md` § Experimental and `VERIFICATION_MODEL.md`.
- `assume(expr)` inside a function body is accepted as an axiom by the
  verifier for that function only (RES-133b) — useful when the caller-side
  precondition isn't visible to the callee's own proof obligation.

---

## 3. Retry-based recovery: `live { }`

```resilient
fn safe_sensor_read() -> int {
    live {
        let reading = read_sensor_with_retry();
        return reading;
    }
}
```

A `live { ... }` block re-runs its body when an `assert(cond, msg)` inside
it fails, instead of terminating the enclosing function. `live_retries()`
inside the block reports the current attempt number (0 on the first pass),
which callee code can use to simulate "succeeds on the Nth attempt"
behavior in tests.

`live invariant COND { ... }` additionally restates a safety invariant that
must hold across every retry; a body that violates it (again via a failing
`assert`) retries rather than continuing with a broken invariant:

```resilient
live invariant count >= 0 && count <= max_count {
    count = count + 1;
    if count > max_count {
        assert(false, "count exceeded max");
    }
}
```

**Backoff and deadlines (RES-138/139/142, Experimental):**
`live backoff(base_ms=10, factor=2, max_ms=1000) { ... }` attaches a delay
between retries; `live backoff(...) within 50ms { ... }` additionally caps
total wall-clock time spent across all retries and their backoff sleeps —
exceeding the budget fails the block instead of retrying forever. A plain
`live { ... }` with no `backoff(...)` clause retries with zero sleep. The
`no_std` runtime does not provide `within`'s wall-clock enforcement (no
clock source is assumed present); see `docs/live-block-semantics.md` for
the full state machine. Keyword parameter names and telemetry counter
names are Experimental — expect them to move (`STABILITY.md` §
Experimental).

`live` blocks are unrelated to the `fails`/`try`/`catch` mechanism above:
`live` recovers by **re-running the same code**, while `try`/`catch`
recovers by **running different code** for a specific declared variant. A
function can nest both.

---

## 4. `Result<T, E>` and `Option<T>`

```resilient
fn safe_divide(int a, int b) -> Option<int> {
    if b == 0 {
        return None;
    }
    return Some(a / b);
}

fn parse_int_from_string(string s) -> Result {
    if len(s) == 0 {
        return Err("invalid format: empty string");
    }
    let parsed = parse_int(s);
    if is_err(parsed) {
        return Err("invalid format: contains non-digit characters");
    }
    return parsed;
}
```

Match on `Option::Some(v)` / `Option::None` and `Result::Ok(v)` /
`Result::Err(e)` explicitly, or use the `is_err`/`unwrap`/`unwrap_err`
builtins. A postfix `?` (RES-086, RES-375) short-circuits both: `expr?`
inside a function returning `Result` unwraps `Ok(v)` to `v` and returns the
`Err(..)` early on failure; inside a function returning `Option`, it
unwraps `Some(v)` to `v` and returns `None` early. `?.` (optional
chaining, RES-363) and `??` (coalescing, RES-375) are separate,
narrower operators for accessing a field/method or supplying a default on
an `Option` value without a full early return — see
`resilient/examples/error_handling_patterns.rz` and `resilient/examples/edge_if_let_pattern.rz`.

This mechanism and the `fails` checked-failure system both express
"this can go wrong," but neither is a strict superset of the other: use
`fails` when the caller must be statically forced to handle (or
re-declare) a specific named failure; use `Result`/`Option` for ordinary
value-level outcomes the caller can freely choose to inspect, ignore, or
pattern-match on.

---

## Effect annotations — parsed, not enforced

The parser accepts a `-e-> TYPE` effect-variable arrow on function
parameter and return types (RES-193), intended to let a higher-order
function's effect classification depend on the effect of a function value
passed to it:

```resilient
fn apply(fn(int) -e-> int f, int x) -e-> int {
    return f(x);
}
```

**This is honestly Experimental and effectively inert today.** The parser
records the effect variable, but effect-polymorphism unification — the
step that would actually classify `apply(pure_fn, ...)` as pure and
`apply(io_fn, ...)` as performing I/O — is not implemented; it is blocked
on a prerequisite chain (an HM-style type walker, generics integration).
Do not treat an `-e->` annotation as a checked or enforced effect boundary;
treat it as a currently-inert placeholder for a future capability. See
`docs/LANGUAGE.md` § Effect system for the same caveat from the language
reference's point of view.

---

## Concurrency and actor failures

Actors (`actor Name { ... }`, RES-332/386/388) attach their own contract
forms: `concurrent_ensures: expr;` (a race-freedom clause the Z3
verifier checks via a commutativity argument), `always: expr;` (a
per-actor safety invariant), and `eventually(after: handler): expr;` (a
bounded-liveness claim). `rz stateright check <file.rz>` model-checks a
**narrow actor-state subset** of the language via a bridge into the
Stateright model checker — it does not model the full language or full
runtime actor semantics, and is explicitly scoped that way in
`stateright_bridge.rs` to avoid overclaiming coverage.

---

## What is *not* a language-level failure mode

Rust-level `panic!`/`unwrap`/`expect` inside the compiler or runtime
implementation are **implementation bugs**, not a Resilient-language
failure primitive — there is no `panic!()` keyword in the language itself.
Per this repository's contribution rules: `resilient-runtime` must have
zero panics in its default `no_std` build (every `unwrap`/`expect` there is
a bug), and the parser/lexer must return a typed `Error` on every failure
path rather than panicking. A `rz`-compiled program's own closest analogue
to "unrecoverable failure" is an unhandled `assert(false, ...)` outside any
`live` block, or an `unwrap()`/`unwrap_err()` builtin call on the wrong
variant — both are runtime errors that terminate the program, and neither
is proved absent by the compiler; write `fails`, `Result`, or a `live`
block instead where recovery matters.

---

## References

- **RES-3505:** Consolidate the failure and recovery semantics (this doc).
- **RES-387 / RES-224 / RES-775:** `fails` declaration, `try`/`catch`
  discharge, runtime checked-failure injection.
- **RES-133b:** `assume()` axioms for `recovers_to` verification.
- **RES-138/139/140/141/142:** `live` backoff/within/escalation semantics.
- **RES-193:** effect-variable arrow (parsed only).
- `docs/live-block-semantics.md` — full `live` block state machine.
- `STABILITY.md` § Experimental — stability status for `live`, effects,
  Z3 verification, and FFI.
- `VERIFICATION_MODEL.md` — what the Z3 verifier actually proves and its
  V1/V2 scope split.
- `MEMORY_MODEL.md` — the compile-time guarantees this document
  deliberately doesn't restate.
