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
- **A-E5 increment 2 (RES-4070):** `check_unannotated_let_alias` (called
  from `check_unannotated_mut_alias`) tracks aliasing through
  straight-line `let` copies of reference bindings: after `let y = x;`
  where `x` is a `&`/`&mut`-typed parameter (or a previous alias of
  one), passing `x` and `y` — or two such copies — to a plain callee's
  reference slots with at least one `&mut` is rejected. The analysis is
  path-sensitive and conservative: branch states merge by
  *intersection* (a fact survives an `if`/loop/match only if it holds
  on every path), assignments and shadowing `let`s *kill* facts rather
  than guessing at re-seating semantics, and match pattern bindings
  kill only the names they can shadow in that arm. A call inside a
  branch is checked against the facts the path reaching it provably
  establishes. This preserves the A-E5 zero-false-positive rule: every
  previously-compiling program keeps compiling unless it provably
  aliases on the executed path.
- **A-E5 increment 3 (RES-4070):** the same alias pass carries
  provenance through a narrow interprocedural summary. A helper with a
  reference return is summarized when every explicit return returns the
  same reference parameter unchanged, including simple conditional or
  match paths. A wrapper may forward through an already-proven helper
  summary, including chains discovered in a fixed point. Mixed
  parameters, unknown wrapper expressions, and nested closure returns
  remain opaque rather than being guessed.
- **A-E5 increment 4 (RES-4070):** declared reference-typed struct fields
  initialized directly from tracked references retain that provenance
  through field reads. Value fields, unknown struct shapes, field writes,
  and arrays remain conservative. Closure capture analysis is described
  below.
- **A-E5 increment 5 (RES-4070):** anonymous function bodies are checked
  with the reference facts captured at their construction site. Captured
  references retain their regions because closures capture the defining
  environment by value; closure parameters shadow captured names and,
  when reference-typed, establish independent roots. The closure's state
  does not flow back into the surrounding function, so creation and later
  invocation remain conservatively separated.
- **A-E5 increment 6 (RES-4070):** direct array literals retain provenance
  for reference elements addressed by non-negative integer literals, so
  `let items = [x]; set_both(x, items[0]);` is rejected when `x` is a
  tracked reference. Dynamic or negative indices, transformed arrays, and
  index writes remain conservative; a known index write kills only that
  element's fact, while an unknown write kills the whole array's facts.
- **A-E5 increment 7 (RES-4070):** constant-bound slices of a tracked array
  retain the provenance of the selected constant-index elements, including
  omitted endpoints and inclusive upper bounds. Dynamic-bound slices and
  other transformations remain opaque.
- **A-E5 increment 8 (RES-4070):** nested declared reference fields retain
  provenance through concrete nested struct literals, so
  `let outer = new Outer { inner: new Inner { item: x } };` followed by
  `set_both(x, outer.inner.item)` is rejected. Unknown struct shapes,
  dynamic paths, and field writes remain conservative.
- **A-E5 increment 9 (RES-4070):** direct array literals of concrete struct
  values retain the same field provenance at constant element paths, so
  `let items = [new Holder { item: x }];` followed by
  `set_both(x, items[0].item)` is rejected. Dynamic indices, transformed
  arrays, and unknown element shapes remain conservative.
- **A-E5 increment 10 (RES-4070):** constant-bound slices of those direct
  struct arrays retain nested field provenance at the rebased element path,
  so `let selected = items[0..1]; set_both(x, selected[0].item)` is also
  rejected. Dynamic-bound slices and transformed sources remain opaque.
- **A-E5 increment 11 (RES-4070):** direct tuple literals retain reference
  provenance at constant tuple-index paths. For example, a pair built from
  a tracked reference still exposes that reference through pair element 0,
  and direct tuple destructuring preserves the same fact. Value elements and
  unknown tuple sources remain conservative.
- **A-E5 increment 12 (RES-4070):** nested direct tuple literals and tuple
  aliases retain reference provenance through chained constant indices. A
  pair shaped as ((x, 0), 1) therefore still exposes x through pair.0.0.
  Tuple values, dynamic indices, and transformed tuple sources remain
  conservative.
- **A-E5 increment 13 (RES-4070):** direct array literals of tuples retain
  the same provenance at constant array and tuple indices. An array shaped as
  [(x, 0)] therefore still exposes x through items[0].0. Dynamic indices,
  transformed arrays, and unknown tuple element shapes remain conservative.
- **A-E5 increment 14 (RES-4070):** constant-bound slices of direct tuple
  arrays retain the same provenance at the rebased array and tuple paths.
  Slicing items[0..1] therefore still exposes x through selected[0].0.
  Dynamic bounds, transformed arrays, and unknown tuple shapes remain
  conservative.
- **A-E5 increment 15 (RES-4070):** direct aliases of already-tracked arrays
  retain known element provenance, including nested struct-field and tuple
  paths. Copying `items` to `copy` therefore still exposes a reference at
  `copy[0]` (or `copy[0].item` / `copy[0].0`) when that path was proven on
  `items`. Dynamic indices, unknown array values, and other transformations
  remain conservative.
- **A-E5 increment 16 (RES-4070):** nested constant array paths retain
  reference provenance through direct nested literals, aliases of tracked
  arrays, and constant-bound outer slices. A matrix shaped as `[[x]]` still
  exposes x through `matrix[0][0]`; dynamic indices, dynamic bounds, and
  transformed or unknown arrays remain conservative.
- **A-E5 increment 17 (RES-4070):** a helper that returns a concrete struct
  with reference-typed fields initialized directly from its reference
  parameters carries those fields' provenance to the caller. Mixed or
  wrapped field initializers, dynamic paths, and ambiguous return shapes
  remain conservative.
- **A-E5 increment 18 (RES-4070):** a helper that returns a direct tuple of
  reference parameters carries each element's provenance to the caller,
  including nested constant tuple paths. Wrapped elements, non-reference
  expressions, and ambiguous return shapes remain conservative.
- **A-E5 increment 19 (RES-4070):** a helper that returns a direct array
  literal carries the provenance of elements initialized directly from its
  reference parameters to the caller. Constant element reads and later array
  aliases continue to expose those facts; wrapped elements, dynamic paths,
  and ambiguous return shapes remain conservative.
- **A-E5 increment 20 (RES-4070):** the same direct array-return summary
  carries proven leaves through constant tuple, struct-field, and nested-array
  paths such as `items[0].0`, `items[0].item`, and `items[0][0]`. Wrapped
  leaves, dynamic/transformed arrays, and ambiguous return shapes remain
  conservative.
- **A-E5 increment 21 (RES-4070):** direct array-return provenance composes
  through a fixed-point chain of already-proven helper calls. A wrapper such
  as `outer(x) -> inner(x)` therefore still exposes a constant path like
  `items[0]`. Only plain reference-parameter forwarding is accepted; recursive,
  wrapped, mixed, and ambiguous returns remain conservative.
- **A-E5 increment 22 (RES-4070):** direct struct- and tuple-return provenance
  composes through the same fixed-point helper chain. Wrappers forwarding a
  known summary therefore preserve paths such as `holder.item` and `pair.0`.
  Recursive, wrapped, mixed, and ambiguous returns remain conservative.
- **A-E5 increment 23 (RES-4070):** helper-returned struct provenance now
  includes nested concrete struct fields, so a returned `Outer` can preserve
  a path such as `value.inner.item` when its `Inner` field is initialized
  directly from a reference parameter. Wrapped nested fields, dynamic paths,
  and ambiguous return shapes remain conservative.
- **A-E5 increment 24 (RES-4070):** tuple-return summaries now preserve the
  same nested struct-field provenance below a constant tuple path, so a
  returned `(Holder, int)` can expose `pair.0.inner.item` when that field is
  initialized directly from a reference parameter. Wrapped, base-updated,
  dynamic, and ambiguous tuple values remain conservative.
- **A-E5 increment 25 (RES-4070):** tuple-return summaries now preserve
  direct array leaves below a constant tuple path, so a returned pair of
  arrays can expose `pair.0[0]` when that element is initialized directly
  from a reference parameter. Wrapped, dynamic, transformed, mixed, and
  ambiguous tuple values remain conservative.
- **A-E5 increment 26 (RES-4070):** tuple-return summaries now compose
  already-proven array-return helpers below a constant tuple path, so a
  wrapper can expose `pair.0[0]` without re-analyzing the helper body. Only
  plain reference-parameter forwarding is accepted; wrapped, transformed,
  mixed, recursive, and ambiguous calls remain conservative.
- **A-E5 increment 27 (RES-4070):** array-return summaries now compose
  already-proven array-return helpers below constant array paths, so a
  wrapper can expose `items[0][0]` without re-analyzing the nested helper.
  Only plain reference-parameter forwarding is accepted; wrapped,
  transformed, mixed, recursive, and ambiguous values remain conservative.
- **A-E5 increment 28 (RES-4070):** direct array literals now compose paths
  from already-proven array-return helper calls placed inside their elements,
  so `let items = [make_array(x)]` exposes `items[0][0]`. Unknown calls and
  non-identifier arguments remain opaque.
- **A-E5 increment 29 (RES-4070):** direct tuple literals now compose paths
  from already-proven tuple-return helper calls placed inside their elements,
  so `let pair = (make_pair(x, y), 0)` exposes `pair.0.0`. Unknown calls and
  non-identifier arguments remain opaque.
- **A-E5 increment 30 (RES-4070):** direct struct literals now compose paths
  from already-proven struct-return helper calls placed inside their fields,
  so `let outer = new Outer { inner: make_inner(x) }` exposes
  `outer.inner.item`. Unknown calls and non-identifier arguments remain opaque.
- **A-E5 increment 31 (RES-4070):** direct tuple literals now compose paths
  from already-proven struct-return helper calls placed inside their elements,
  so `let pair = (make_inner(x), 0)` exposes `pair.0.item`. Unknown calls and
  non-identifier arguments remain opaque.
- **A-E5 increment 32 (RES-4070):** direct tuple literals now compose paths
  from already-proven array-return helper calls placed inside their elements,
  so `let pair = (make_array(x), 0)` exposes `pair.0[0]`. Unknown calls and
  non-identifier arguments remain opaque.
- When the syntactic signature-level rule rejects a program, a Z3
  fallback using the function's `requires` preconditions may still
  accept it (RES-393 D1), if the `z3` feature is enabled. The new A-E5
  check has no Z3 fallback yet — see the "What does not exist" list.

**What does not exist (yet)** (tracked in
[#4070](https://github.com/EricSpencer00/Resilient/issues/4070)):
- No use-after-move detection for unannotated (non-`linear`) bindings,
  **and none is planned** — see
  [`COPY_MOVE.md`](COPY_MOVE.md) (RES-4079). Every non-`linear`,
  non-reference type is Copy by design decision, matching what the
  interpreter has always done; `linear T` remains the sole
  move-semantics surface, enforced by `check_linear_usage`
  (`resilient/src/linear.rs`).
- Conditional-path aliasing detection is *partial*. The let-alias pass
  above handles `if`/`while`/`for`/`match` path merging by intersection,
  but there is no Z3-backed branch-condition disjointness reasoning.
  Alias facts established by dynamic or transformed array elements remain
  invisible; known struct-field facts are limited to concrete direct
  literals (including nested declared fields and direct array-literal
  elements and their constant-bound slices) and are killed on field writes.
  Closure bodies are checked using captured
  facts, while array tracking is limited to direct literals, direct aliases of
  their known paths, constant index paths (including nested array paths), and
  constant-bound slices.
- No general whole-program or interprocedural alias analysis. The pass
  has only the narrow direct-reference, concrete-struct, tuple, and direct
  array return summaries described above; it does not track references through
  statics, dynamically transformed array elements, or ambiguous return paths,
  and struct-field tracking remains limited to direct literals or those proven
  struct returns.
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
