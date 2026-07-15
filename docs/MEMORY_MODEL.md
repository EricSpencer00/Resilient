# Resilient Memory Model

## Overview

Resilient's memory model defines how memory is allocated, accessed, and guaranteed to be safe across the compiler, runtime, and embedded targets. This document specifies the allocation tiers, aliasing rules, and mutual exclusivity guarantees that enable safe-critical systems programming.

> **Reconcile-to-reality note (RES-3504.1, 2026-07):** the sections below
> describe the memory model's *design target*. Several sub-sections read
> as unconditional compiler guarantees; in the current implementation
> some of them are enforced narrowly or not yet enforced at all. See
> [Enforcement Reality Check](#enforcement-reality-check-what-is-actually-checked-today)
> before relying on any claim in this document for a safety argument.

---

## Allocation Tiers

Memory in Resilient is organized into four distinct tiers, each with different safety guarantees and use cases.

### Tier 1: Stack Allocation

**Characteristics:**
- Automatic allocation on function entry
- Automatic deallocation on function exit
- LIFO (last-in-first-out) ordering
- Bounded size known at compile time
- Zero runtime overhead

**Safety Guarantees:**
- All accesses are valid within the function scope
- No use-after-free possible
- No memory leaks possible
- Aliasing is permitted but tracked by the type system

**Usage:**
```rust
fn process(int x) -> int {
    let local = x + 1;  // Stack allocated
    return local;       // Automatically freed on return
}
```

**Constraints:**
- Size must be compile-time constant
- Variable-length arrays require explicit heap allocation
- Recursion is safe but bounded by stack depth

---

### Tier 2: Static Allocation

**Characteristics:**
- Allocated in the binary's data section
- Lifetime is the entire program execution
- Accessible from any function
- Zero runtime allocation cost
- Address known at compile time on most targets

**Safety Guarantees:**
- Always accessible (never deallocated)
- Address never changes
- Safe to store as constant pointers
- Concurrency-safe if declared as immutable

**Usage:**
```rust
static CONFIG = [1, 2, 3, 4, 5];

fn read_config(int index) -> int {
    return CONFIG[index];  // Always safe
}
```

**Constraints:**
- Size must be compile-time constant
- Initialization must be constant expressions
- Mutable static requires explicit synchronization for concurrency

---

### Tier 3: Heap Allocation

**Characteristics:**
- Dynamically allocated at runtime
- Size determined at runtime
- Lifetime managed by programmer or runtime GC
- May require deallocation
- Available only with `#[cfg(feature = "alloc")]`

**Safety Guarantees:**
- Bounds checked on every access
- Type information preserved
- Lifetime tracked (when using reference semantics)
- No double-free with proper ownership rules

**Usage:**
```rust
#[cfg(feature = "alloc")]
fn process_array(int count) -> array<int> {
    let data = allocate::<int>(count);
    for i in 0..count {
        data[i] = i * 2;
    }
    return data;  // Ownership transferred
}
```

**Constraints:**
- Requires allocator (unavailable in strict `no_std`)
- Lifetime rules must be followed to prevent use-after-free
- Allocation may fail (returns Option or Result)

**Runtime grounding:** `resilient-runtime/src/lib.rs` is
`#![cfg_attr(not(any(test, feature = "std-sink")), no_std)]` and gates
every `Vec`/heap-backed code path behind `#[cfg(feature = "alloc")]`
(e.g. lines 31, 50, 98, 124, 152, 210, 322, 334, 351). A `static-only`
feature additionally exists for targets that want static allocation
but must reject `alloc` entirely (`#[cfg(all(feature = "static-only",
not(feature = "alloc")))]`), which is the concrete mechanism behind the
"unavailable in strict `no_std`" constraint above.

---

### Tier 4: MMIO Allocation

**Characteristics:**
- Memory-mapped I/O registers on embedded systems
- Address fixed by hardware specification
- Accessed via volatile reads/writes
- Lifetime is entire program (hardware register)
- Platform-specific

**Safety Guarantees:**
- Address is guaranteed by hardware specification
- Access ordering is preserved (volatile semantics)
- Type information ensures correct register widths
- Safe concurrent access with proper synchronization

**Usage:**
```rust
#[mmio(base = "0x40010800", size_bytes = "0x400")]
struct GPIOA {
    #[bits(0..=15), rw]
    mode: u16,
    #[bits(16..=31), ro]
    status: u16,
}
```

**Constraints:**
- Address must be valid for the target hardware
- Access width must match hardware specification
- Volatile semantics prevent compiler optimizations that would change timing

---

## Aliasing Rules

Resilient follows Rust-like ownership and borrowing rules to prevent data races and use-after-free bugs. **This section describes the design target.** The compiler pass that implements it today (`region_inference.rs` + `check_region_aliasing` in `lib.rs`, RES-391/RES-394) is a narrow, syntactic MVP — see [Enforcement Reality Check](#enforcement-reality-check-what-is-actually-checked-today) for exactly what it does and does not catch.

### Exclusive Access (Mutable References)

Only one mutable reference to a value may exist at a time:

```rust
fn modify(data: &mut array<int>) {
    // This is the only way to access `data`
    // No other references can exist
    for i in 0..data.len() {
        data[i] = data[i] + 1;
    }
}
```

### Shared Access (Immutable References)

Multiple immutable references may coexist:

```rust
fn read_multiple(data: &array<int>) -> int {
    // Multiple readers can call this simultaneously
    // No writer can exist while readers are active
    return data[0] + data[1];
}
```

### No Dangling Pointers

References cannot outlive their referents:

```rust
fn safe_borrow(x: int) -> &int {
    return &x;  // ✗ ERROR: `x` is deallocated on return
}

fn safe_return() -> &static int {
    return &STATIC_VALUE;  // ✓ OK: static lifetime
}
```

---

## Mutability Semantics

### Immutable by Default

```rust
let x = 5;      // x is immutable - cannot change
x = 10;         // ✗ ERROR
```

### Explicit Mutability

```rust
let mut x = 5;  // x is mutable - can change
x = 10;         // ✓ OK
```

### Interior Mutability for Concurrency

```rust
static COUNTER = cell<int>(0);

fn increment() {
    COUNTER.set(COUNTER.get() + 1);  // Safe mutation of static
}
```

---

## Guarantees Across Feature Tiers

| Guarantee | `std` | `no_std` | `no_std` + `alloc` | Embedded MMIO |
|-----------|-------|----------|-------------------|---------------|
| Stack allocation | ✅ | ✅ | ✅ | ✅ |
| Static allocation | ✅ | ✅ | ✅ | ✅ |
| Heap allocation | ✅ | ❌ | ✅ | ❌ |
| MMIO access | ⚠️ | ✅ | ⚠️ | ✅ |
| Concurrency | ✅ | ✅ | ✅ | ✅ |

**Correction:** an earlier revision of this table listed a "GC (garbage
collection)" row claiming `std` builds have a garbage collector. Resilient
has **no garbage collector in any configuration** — `grep -rn
"garbage.collect\|GcCollect" resilient/src resilient-runtime/src` returns
nothing. Heap values use ownership/move semantics (RES-3504 target) with no
tracing or reference-counted collector implemented. The row has been
removed rather than corrected to "❌" across the board, since a row that
is always false everywhere isn't informative.

---

## Memory Safety Invariants

These are the invariants the memory model is designed to guarantee. Item
6 (bounds safety) is enforced today by a dedicated compiler pass
(`bounds_check::check_array_bounds`, gated on `markers.has_index_expression`
in `typechecker.rs`). Items 1–4 are **design invariants, not yet fully
enforced** — see the next section for the precise scope of the aliasing
checker that exists today. Treat 1–4 as the target this document is
scoping work toward, not a guarantee you can rely on for a safety case.

1. **No use-after-free**: References cannot access deallocated memory
2. **No double-free**: Memory is freed exactly once
3. **No dangling pointers**: References do not outlive their referents
4. **No data races**: Exclusive access prevents simultaneous mutations
5. **Type safety**: All memory accesses respect type constraints
6. **Bounds safety**: All array/slice accesses are in bounds ✅ enforced

---

## Enforcement Reality Check: what is actually checked today

Grounded in `resilient/src/region_inference.rs` and the
`check_region_aliasing` pass in `resilient/src/lib.rs`
(RES-391/RES-393/RES-394/RES-395, A-E5 · #3933).

**What exists:**
- A **syntactic, function-signature-level** aliasing check. For every
  top-level `fn`, it looks at the reference-typed parameters (`&T`,
  `&mut T`, optionally with a `[LABEL]` region annotation) and rejects a
  pair of `&mut` parameters that *could* alias: same declared region
  label, or one labeled and one unlabeled.
- Unlabeled `&mut` parameters get inference-assigned region variables
  (`region_inference::build_region_map`); two unlabeled `&mut` params
  with distinct inferred regions are accepted as independent (RES-394 D5).
- `region_inference::check_call_site_region_aliasing` checks call-site
  region-label consistency for **region-polymorphic** callees
  (`fn f<R, S>(...)`) — it substitutes each type-param region with the
  caller's concrete label and rejects a call that unifies two `&mut`
  parameters onto the same region.
- **A-E5:** `region_inference::infer` (backed by
  `check_unannotated_mut_alias`) is no longer a no-op. It closes the gap
  the call-site check above cannot: a **plain, non-generic** function
  whose `&mut` parameters carry no `[LABEL]` at all. Within a single
  call expression, if the same identifier is passed as the argument for
  two (or more) parameter slots and at least one of those slots is
  `&mut`, the two references are provably the same runtime binding —
  this needs no region-label inference, only syntactic identity within
  one call's argument list, so it is unconditionally sound (no false
  positive is possible). Region-polymorphic callees are left to the
  call-site-substitution check above to avoid double-reporting.
- When the syntactic signature-level rule rejects a program, a Z3
  fallback using the function's `requires` preconditions may still
  accept it (RES-393 D1), if the `z3` feature is enabled. The new A-E5
  check has no Z3 fallback yet — see the "What does not exist" list.

**What does not exist (yet)** (tracked in
[#4070](https://github.com/EricSpencer00/Resilient/issues/4070)):
- No use-after-move detection for unannotated (non-`linear`) bindings.
  The language has no Copy/Move type distinction outside `linear T`
  (`resilient/src/linear.rs`), so there is no sound way yet to tell
  whether re-reading a plain local after passing it somewhere is a
  genuine violation or an ordinary value copy.
- No conditional-path aliasing detection. The A-E5 call-site check only
  catches literal syntactic identifier repetition within one call's
  argument list (straight-line); a program that aliases the same
  binding only on some branches is not caught.
- No whole-program or interprocedural alias analysis. Both region
  checks only look at parameter *signatures* and direct call-site
  arguments; neither tracks whether a reference escapes into a struct
  field, a static, a return value, an array element, or a closure
  capture.
- No borrow checker over local-to-local aliasing — there is no
  expression syntax in the language today to take a reference to
  another local (`&mut` only ever appears in parameter/`let` *type*
  annotations, never as an expression), so this has no concrete surface
  to check yet.
- No lifetime/region tracking beyond the function-parameter boundary
  (there is no equivalent of Rust's NLL or region-based lifetime
  elaboration across a whole function body).
- No enforcement for Tier 3 (Heap) or Tier 4 (MMIO) aliasing beyond
  whatever the `&`/`&mut` parameter and call-site checks happen to cover
  if a heap/MMIO reference is passed as a parameter.

**Practical implication:** the "Aliasing Rules" and "Memory Safety
Invariants" sections above describe the *intended* end-state model. Code
that violates the invariants informally (e.g., stores a `&mut`
reference to a struct field and reads it through a second alias that
never appears as a function parameter pair, or aliases a variable only
on one branch of an `if`) will compile today without error. Do not cite
this document as evidence of a memory-safety guarantee beyond
bounds-checking and the narrow parameter-signature-level and
direct-call-site aliasing rules described above.

---

## Embedded Target Specifics

### Cortex-M (ARM Embedded)

- Stack: SRAM with size known at link time
- Static: Flash or SRAM (zero-initialized BSS section)
- MMIO: Memory-mapped peripherals at fixed addresses
- Guarantees: Full memory safety with predictable timing

### RISC-V

- Stack: Similar to Cortex-M
- Static: Program flash and RAM regions
- MMIO: Device-specific register maps
- Guarantees: Full memory safety, configurable MPU regions

---

## Examples: Safe Patterns

### Pattern 1: Stack-Allocated Buffer

```rust
fn process_frame(int size) -> int {
    let buffer: array<int>(256) = [0..256];  // Stack alloc, bounded
    for i in 0..size {
        buffer[i] = i * 2;
    }
    return buffer[0];  // Auto-freed on return
}
```

### Pattern 2: Static Configuration

```rust
static DEVICE_CONFIG = {
    address: 0x40010800,
    timeout: 1000,
    flags: 0xABCD,
};

fn get_timeout() -> int {
    return DEVICE_CONFIG.timeout;
}
```

### Pattern 3: Mutable Reference

```rust
fn zero_array(data: &mut array<int>) {
    for i in 0..data.len() {
        data[i] = 0;
    }
}

fn main() {
    let mut arr = [1, 2, 3, 4, 5];
    zero_array(&mut arr);  // Exclusive borrow
}
```

---

## Next Steps

Follow-up PRs will:
1. Enforce memory tier constraints in the compiler
2. Add explicit lifetime annotations for complex references
3. Implement runtime bounds checking with zero-cost optimizations
4. Define unsafe blocks and preconditions for system programming

---

## References

- **RES-3504**: Specify and enforce the memory model
- **RES-3501**: Stabilize the language reference and feature-tier policy
- **LANGUAGE.md**: Feature tier classification framework
