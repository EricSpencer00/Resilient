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

## Phase 1: Generics (Planned: v0.3–v0.4)

**Status:** Design stage (RES-3503, RES-3502)  
**Dependency:** Stable syntax and parsing infrastructure  
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

### 1.5 Phase 1 Deliverables

- [ ] Lexer support for `<T>` syntax
- [ ] Parser for generic declarations
- [ ] Type checker for generic constraints
- [ ] Monomorphization in code generation
- [ ] Compiler tests: 50+ cases
- [ ] Examples: generic list, generic pair, generic map function
- [ ] Documentation: this roadmap + language reference section

---

## Phase 2: Effects System (Planned: v0.5–v0.6)

**Status:** Design stage  
**Dependency:** Phase 1 (Generics) must be stable  
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

| Feature | Phase 1 | Phase 2 | Phase 3 |
|---------|---------|---------|---------|
| Generic functions | ✅ | ✅ | ✅ |
| Generic types | ✅ | ✅ | ✅ |
| Generic constraints | ✅ | ✅ | ✅ |
| Effect annotations | ❌ | ✅ | ✅ |
| Effect polymorphism | ❌ | ✅ | ✅ |
| Local type inference | ❌ | ❌ | ✅ |
| Effect inference | ❌ | ❌ | ✅ |
| Bidirectional checking | ❌ | ❌ | ✅ |

---

## Current State vs. Future

### Today (v0.2.x)

```rust
// No generics — must repeat for each type
fn swap_int(a: int, b: int) -> (int, int) {
    return (b, a);
}

fn swap_string(a: string, b: string) -> (string, string) {
    return (b, a);
}

// No effects — all functions are implicitly pure
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
| Phase 1 Design (RES-3503) | v0.3 | 📋 Design phase |
| Phase 1 Implementation | v0.3–v0.4 | ⏳ Blocked on RES-3502 |
| Phase 2 Design | v0.4–v0.5 | 📋 Design phase |
| Phase 2 Implementation | v0.5–v0.6 | ⏳ After Phase 1 |
| Phase 3 Design | v0.6+ | 📋 Future |
| Phase 3 Implementation | v0.7+ | ⏳ After Phase 2 |

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
**Resolution:** Required before Phase 1 code generation.

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
