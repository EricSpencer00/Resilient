# Resilient Memory Model

## Overview

Resilient's memory model defines how memory is allocated, accessed, and guaranteed to be safe across the compiler, runtime, and embedded targets. This document specifies the allocation tiers, aliasing rules, and mutual exclusivity guarantees that enable safe-critical systems programming.

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

Resilient follows Rust-like ownership and borrowing rules to prevent data races and use-after-free bugs.

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
| GC (garbage collection) | ✅ | ❌ | ❌ | ❌ |

---

## Memory Safety Invariants

These invariants are maintained by the compiler and enforced at runtime:

1. **No use-after-free**: References cannot access deallocated memory
2. **No double-free**: Memory is freed exactly once
3. **No dangling pointers**: References do not outlive their referents
4. **No data races**: Exclusive access prevents simultaneous mutations
5. **Type safety**: All memory accesses respect type constraints
6. **Bounds safety**: All array/slice accesses are in bounds

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
