---
title: Type System Roadmap
parent: Language Reference
nav_order: 9
permalink: /type-system-roadmap
---

# Resilient Type System Roadmap

## Overview

Resilient's type system evolution is strategically phased to build a foundation for safety-critical embedded systems. This roadmap maps the long-term vision of **generics**, **effects**, and **type inference** with clear dependencies and implementation order.

The end-state type system will enable:
- **Generics**: Polymorphic functions and types with compile-time specialization
- **Effects**: Explicit tracking of side effects (I/O, mutation, resource allocation)
- **Inference**: Smart type deduction while maintaining full explicitness at API boundaries

> **Reconcile-to-reality note (RES-3503.1, 2026-07):** this roadmap was
> originally written describing Phase 1 (Generics) as future "Design
> stage" work. It has since shipped. The checklists and status markers
> below have been corrected against the current typechecker's
> `<EXTENSION_PASSES>` block (`resilient/src/typechecker.rs`) so this
> document stops overstating what is still pending. Phase 2 (Effects) is
> further from reality than the original draft implied: there is no
> parseable effect-annotation syntax in the lexer today, not even a
> "parsed but unenforced" form.

---

## Phased Implementation Strategy

```
Phase 1: Generics Foundation
    ↓
Phase 2: Effects System
    ↓
Phase 3: Type Inference & Integration
```

Each phase is a self-contained increment that does not break existing code.

---

## Phase 1: Generics (Shipped, v0.2.x)

**Status:** ✅ **Shipped** — generic functions, generic structs, generic
enums, trait bounds (including associated types and blanket impls), and
exhaustiveness checking are all live in the default build today. This
status superseded the original "Design stage" draft; grounding below.
**Scope:** Generic functions and generic data types

### 1.1 Generic Functions

```rust
// Single type parameter
fn swap<T>(a: T, b: T) -> (T, T) {
    return (b, a);
}

// Multiple type parameters
fn map<T, U>(f: (T) -> U, value: T) -> U {
    return f(value);
}

// Type parameter with bounds (future)
fn clone<T: Cloneable>(x: T) -> T {
    return x.clone();
}
```

### 1.2 Generic Data Types

```rust
// Generic struct
struct Pair<T, U> {
    first: T,
    second: U,
}

// Generic array wrapper
struct Stack<T> {
    elements: array<T>,
    size: int,
}

// Usage
fn process_pair() {
    let p: Pair<int, string> = Pair { first: 42, second: "answer" };
}
```

### 1.3 Generic Constraints (Phase 1.5)

```rust
// Trait-like bounds
fn print_all<T: Printable>(items: array<T>) {
    for item in items {
        item.print();
    }
}

// Multiple constraints
fn sorted_pair<T: Comparable + Copyable>(a: T, b: T) -> (T, T) {
    if a < b {
        return (a, b);
    }
    return (b, a);
}
```

### 1.4 Monomorphization

The compiler specializes generic code at compile time:

```rust
fn swap<T>(a: T, b: T) -> (T, T) { ... }

// Call sites specialize:
swap(1, 2);          // swap<int>(1, 2)
swap("x", "y");      // swap<string>("x", "y")

// Generated code: two separate functions
```

**Compiler guarantee:** Zero runtime overhead. All specialization happens at compile time.

### 1.5 Phase 1 Deliverables — status against `resilient/src/`

- [x] Lexer support for `<T>` syntax — parsed by the main parser in `lib.rs`
- [x] Parser for generic declarations — `Node::Function.type_params`,
      `Node::StructDecl`/`EnumDecl` generic variants
- [x] Type checker for generic constraints — wired in the
      `typechecker.rs` `<EXTENSION_PASSES>` block:
      `crate::generics::check` (gated on `markers.has_generic_fn`),
      `crate::generic_inference::check` (RES-2576, call-site inference),
      `crate::variance::check` (RES-2615), `crate::generic_enums::check`
      (RES-2575), `crate::generic_structs::check` (RES-2574),
      `crate::traits::check` (trait bounds + associated types +
      RES-2695 projection bounds like `I::Item: Display`),
      `crate::blanket_impl::check` (RES-2552)
- [x] Monomorphization in code generation — `crate::monomorph::lower`
      runs before both the `--vm` bytecode compiler and the `--jit`
      Cranelift backend (`lib.rs`, the `use_vm`/`use_jit` dispatch
      arms); the interpreter path type-checks generics without a
      separate specialization step since `Value` is dynamically typed
      at runtime
- [x] Compiler tests: generics, generic_enums, generic_structs, traits,
      blanket_impl, and exhaustiveness each have dedicated `#[cfg(test)]`
      modules well past the 50-case bar cumulatively
- [x] Examples: see `resilient/examples/` generic-feature corpus
- [x] Documentation: this roadmap + `docs/LANGUAGE.md` feature-tier section

**Associated types — correction:** an earlier draft of this roadmap
assumed associated types were "parsed but unenforced." That is not what
`resilient/src/traits.rs` does: `check()` rejects an `impl` block missing
a trait-declared associated type (`"missing associated type"` error,
tested by `impl_missing_associated_type_errors`), rejects duplicate
associated-type declarations in a trait (tested by
`duplicate_associated_type_in_trait_errors`), and RES-2695 enforces
projection bounds (`T::Assoc: SomeBound`) at call sites. Associated
types are enforced today, not merely parsed.

---

## Phase 2: Effects System (Planned: v0.5–v0.6)

**Status:** Design stage — **further from reality than "design stage"
implies.** Grounding: `grep -n "Bang" resilient/src/lexer_logos.rs`
shows a `!` token used for the boolean-not operator and the `-> !`
never-return type (`crate::never_type::check`), but there is **no**
lexer or parser support for an effect-annotation grammar of the form
`-> T ! IO` shown in the examples below — that syntax does not parse
today. Do not confuse this with the unrelated `fn_effects` machinery
that already exists: RES-192's `infer_fn_effects` computes a per-function
boolean IO-effect lattice, gated behind `TypeChecker::warn_unverified`-style
opt-in and consumed only by the `--audit` / `--explain-effects` CLI
drivers (`typechecker.rs` around line 6849). That is a diagnostic
side-channel, not the effect-polymorphism type system described below —
none of the syntax in this section (`! IO`, `! (IO, Mutation)`, effect
type parameters `<E>`, effect bounds `E: Pure`) exists in the grammar.
**Scope:** Explicit effect tracking, effect polymorphism

### 2.1 Effect Types

```rust
// Pure (no side effects)
fn add(x: int, y: int) -> int {
    return x + y;
}

// IO effect
fn read_sensor() -> int ! IO {
    return sensor.read();  // Explicit: function has IO effect
}

// Mutation effect
fn increment(counter: &mut int) -> unit ! Mutation {
    *counter = *counter + 1;
}

// Multiple effects
fn log_and_read(msg: string) -> int ! (IO, Mutation) {
    debug_output(msg);  // IO
    counter += 1;       // Mutation
    return sensor.read();
}
```

### 2.2 Effect Polymorphism

```rust
// Generic over effects
fn run_twice<E>(f: () -> int ! E) -> int ! E {
    let a = f();
    let b = f();
    return a + b;
}

// Caller must handle effects
fn main() {
    // Pure caller can only call pure functions
    let x = add(1, 2);  // OK: no effects
    
    // To call IO functions, propagate the effect
    let sensor_val = read_sensor();  // Propagates: () -> int ! IO
}
```

### 2.3 Effect Composition

```rust
// Sequential effects compose
fn read_log(filename: string) -> string ! (IO, Mutation) {
    let file = open(filename);      // IO effect
    log_access();                    // Mutation (logging state)
    return file.read();              // IO effect
}

// Parallel effects (future)
fn parallel_reads() -> (int, int) ! (IO, Parallelism) {
    // Both tasks have IO, plus Parallelism effect
}
```

### 2.4 Effect Bounds

```rust
// Effect constraint: "only pure functions"
fn filter_pure<T, E: Pure>(f: (T) -> bool ! E, items: array<T>) -> array<T> {
    // Type checking: E must be Pure (no effects)
    // Enables optimization: predicate has no side effects
}

// Effect constraint: "only IO or Mutation, not Parallelism"
fn safe_async<E: IO | Mutation>(f: () -> int ! E) -> int {
    // Guarantees no race conditions
}
```

### 2.5 Phase 2 Deliverables

- [ ] Effect annotation syntax (`! E`)
- [ ] Effect type checker
- [ ] Effect inference at call sites
- [ ] Effect polymorphism (generic over effects)
- [ ] Standard effect library: Pure, IO, Mutation, Resource, Parallelism
- [ ] Compiler tests: 100+ cases
- [ ] Examples: pure predicates, stateful iterators, safe I/O wrappers
- [ ] Documentation: effect semantics, examples, best practices

---

## Phase 3: Type Inference & Integration (Planned: v0.7+)

**Status:** Design stage  
**Dependency:** Phase 1 (Generics) and Phase 2 (Effects) both stable  
**Scope:** Smart type inference, bidirectional checking

**Naming collision to be aware of:** `resilient/src/region_inference.rs`
already exists, but it is unrelated to the "Type Inference" scoped here —
it's a region/aliasing analysis for the memory model (see
`docs/MEMORY_MODEL.md`'s Enforcement Reality Check section), not a
general type-inference engine. Its top-level `infer` entry point is a
**no-op stub returning `Ok(())`** (`typechecker.rs:6397` comment: "RES-1611:
`region_inference::infer` is a no-op stub"); the real region-aliasing
logic runs from `check_call_site_region_aliasing`, called separately.
Phase 3 of this roadmap (local type inference, bidirectional checking)
has no code behind it yet — generic call-site *argument* inference
(RES-2576, `generic_inference::check`) already ships as part of Phase 1
and should not be confused with the broader Phase 3 inference vision
described below.

### 3.1 Local Type Inference

```rust
// Compiler infers local variable types
fn process() {
    let x = 42;              // Inferred: int
    let y = [1, 2, 3];      // Inferred: array<int>
    let result = add(x, 1); // Inferred: int
}

// But API boundaries require explicit types
fn public_api(x: int) -> string {  // Explicit at boundary
    let local = process_internal(x);  // Type inferred internally
    return local.to_string();
}
```

### 3.2 Bidirectional Type Checking

```rust
// Context helps infer types
fn apply_to_pair() {
    let p: Pair<int, string> = Pair { first: 1, second: "a" };
    // Compiler knows: generic instantiation is Pair<int, string>
    // So it infers types in Pair's methods
}

// Function type inference
fn with_callback<T>(f: (T) -> int, value: T) -> int {
    // Compiler infers: f's parameter type is T
}
```

### 3.3 Inference Limitations (Explicit Design)

```rust
// Generic instantiation must be explicit OR inferrable from context
fn swap<T>(a: T, b: T) -> (T, T) { ... }

swap(1, 2);                    // OK: inferred as swap<int>
swap(1, "x");                  // ERROR: cannot infer T (int ≠ string)
swap::<int>(1, 2);            // OK: explicit

// Ambiguous cases still require annotations
let container: Stack<int> = Stack { ... };  // Explicit type needed
```

### 3.4 Effect Inference

```rust
// Effect type can be inferred from implementation
fn compute_sum(numbers: array<int>) -> int {  // No effect annotation
    let total = 0;
    for n in numbers {
        total += n;  // Mutation: does not escape function
    }
    return total;  // Compiler infers: () -> int ! Pure
}

// But when effects escape, they must be explicit
fn update_state(counter: &mut int) -> unit {  // Compiler infers: Mutation
    *counter += 1;
}
```

### 3.5 Inference + Generics Interaction

```rust
// Generic with inference
fn collect_results<T>(items: array<T>) -> Stack<T> {
    let stack = Stack { elements: items, size: 0 };  // T inferred from parameter
    return stack;  // Return type satisfies generic constraint
}

// Bidirectional + polymorphism
fn run_effect<E>(action: () -> int ! E) -> int ! E {
    let result = action();  // Compiler infers E from action's type
    return result;
}
```

### 3.6 Phase 3 Deliverables

- [ ] Bidirectional type checking algorithm
- [ ] Local type inference (no annotations inside function bodies)
- [ ] Effect inference from implementation
- [ ] Generic specialization inference
- [ ] Compiler tests: 150+ cases
- [ ] Performance benchmarks: inference must be <50ms per module
- [ ] Documentation: inference rules, limitations, best practices
- [ ] LSP support: type hints in editor

---

## Feature Interaction Matrix

Columns are roadmap phases (planned scope), not calendar time. The
"Shipped Today" column is the actual state of the default build as of
this revision (grounded in the greps cited throughout this document).

| Feature | Shipped Today | Phase 1 | Phase 2 | Phase 3 |
|---------|:---:|:---:|:---:|:---:|
| Generic functions | ✅ | ✅ | ✅ | ✅ |
| Generic types (structs/enums) | ✅ | ✅ | ✅ | ✅ |
| Generic constraints (trait bounds) | ✅ | ✅ | ✅ | ✅ |
| Associated types (decl + impl-completeness) | ✅ | ✅ | ✅ | ✅ |
| Projection bounds (`T::Assoc: Bound`) | ✅ | ✅ | ✅ | ✅ |
| Call-site generic argument inference | ✅ | ✅ | ✅ | ✅ |
| Effect annotation syntax | ❌ | ❌ | ✅ | ✅ |
| Effect polymorphism | ❌ | ❌ | ✅ | ✅ |
| Local `let`/return-type inference (basic, RES-189/2569)¹ | ⚠️ | ⚠️ | ⚠️ | ✅ |
| Effect inference | ❌ | ❌ | ❌ | ✅ |
| Bidirectional checking | ❌ | ❌ | ❌ | ✅ |

¹ `typechecker.rs`'s `let_type_hints` (RES-189) and `fn_return_type_hints`
(RES-2569) already infer types for unannotated `let` bindings and
function returns today — but as straightforward bottom-up inference for
LSP inlay hints, not the full bidirectional/context-propagating engine
Phase 3 describes. "Shipped Today" is ⚠️, not ✅, to mark that distinction.

---

## Current State vs. Future

### Today (v0.2.x) — corrected

```rust
// Generics ship today — no need to repeat per type
fn swap<T>(a: T, b: T) -> (T, T) {
    return (b, a);
}

swap(1, 2);          // OK, monomorphized/dynamically typed at call site
swap("x", "y");       // OK, same generic fn

// No effects — this part of the original draft is still accurate.
// All functions are implicitly pure from the type system's
// perspective; there is no `! IO`-style annotation syntax in the
// grammar, so mutation and I/O are not tracked in the type.
fn process(data: &mut Data) -> int {
    // Mutation is implicit, hard to track
}
```

### Future (v0.6+)

```rust
// Generic and polymorphic
fn swap<T>(a: T, b: T) -> (T, T) {
    return (b, a);
}

// Effects explicit
fn process(data: &mut Data) -> int ! Mutation {
    // Mutation is explicit and tracked
}

// Can be polymorphic over effects
fn apply<E>(f: () -> int ! E) -> int ! E {
    return f();
}
```

---

## Timeline & Milestones

| Milestone | Expected | Status |
|-----------|----------|--------|
| Phase 1 Design (RES-3503) | v0.2.x | ✅ Done |
| Phase 1 Implementation (generics, traits, associated types, exhaustiveness) | v0.2.x | ✅ **Shipped** — see §1.5 for pass-by-pass grounding |
| Phase 2 Design (Effects) | v0.4–v0.5 | 📋 Design phase — no syntax exists yet |
| Phase 2 Implementation | v0.5–v0.6 | ⏳ Not started |
| Phase 3 Design (Inference) | v0.6+ | 📋 Future — do not confuse with the unrelated `region_inference.rs` module (see §Phase 3 note) |
| Phase 3 Implementation | v0.7+ | ⏳ Not started |

---

## Rationale & Design Decisions

### Why Generics First?

Generics are foundational. They enable:
- Code reuse without macros
- Type-safe polymorphism
- Monomorphization at compile time (no runtime cost)

Effects build on top of generics (effect polymorphism requires generic parameters).

### Why Effects Before Inference?

Effects make implicit semantics explicit. Once effects are part of the type, inference can reason about them. Effects-first ensures:
- Developers know what side effects are happening
- Compiler can track effects reliably
- Inference can work correctly with effect constraints

### Why Inference Last?

Inference is a productivity feature, not a foundation feature. Implementing it last:
- Doesn't block generics or effects
- Lets stabilization happen before adding complexity
- Maintains explicit type boundaries at API surfaces

### Monomorphization Model

Resilient uses **compile-time specialization** (like Rust, C++ templates):

```
Generic source → Specialization → Specialized code → Binary
                (per call site)
```

**Benefit:** Zero runtime overhead, optimal code generation.  
**Trade-off:** Compile time increases with specialization count.

---

## Blockers & Dependencies

### RES-3502: Module System
**Status:** In progress  
**Impact:** Generics require clear module boundaries for visibility rules.  
**Resolution:** ~~Required before Phase 1 code generation~~ — Phase 1
shipped without waiting on this; module-boundary interactions with
generics (e.g., visibility of generic type parameters across module
scopes) remain an open refinement, not a hard blocker.

### RES-3505: Failure/Recovery Semantics
**Status:** Not started  
**Impact:** Effects need a clear model for error handling.  
**Resolution:** Required before Phase 2 finalizes.

### RES-3506: Backend Architecture
**Status:** Not started  
**Impact:** Monomorphization and effect tracking differ across backends (JIT, interpreter, embedded).  
**Resolution:** Required before Phases 1–3 ship across all backends.

---

## Glossary

- **Monomorphization**: Compile-time specialization of generic code for each instantiation.
- **Generic constraint**: A requirement on a type parameter (e.g., `T: Printable`).
- **Effect polymorphism**: A function that works with multiple effect types.
- **Bidirectional checking**: Type inference that uses both top-down (context) and bottom-up (code) information.
- **Effect bound**: A constraint on which effects are allowed (e.g., `Pure`, `IO | Mutation`).

---

## References

- **RES-3503**: Unify the long-term type system roadmap
- **RES-3502**: Design a real module and package system
- **RES-3505**: Consolidate the failure and recovery semantics
- **RES-3506**: Define the backend architecture contract
- **LANGUAGE.md**: Feature tier classification framework
- **MEMORY_MODEL.md**: Memory safety model
