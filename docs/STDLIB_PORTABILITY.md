---
title: Standard Library Portability
parent: Language Reference
nav_order: 7
permalink: /stdlib-portability
---

# Resilient Standard Library Portability Model

## Overview

Resilient's standard library is organized into portability tiers that enable code to be written once and deployed across desktop, embedded, and constrained environments. This document specifies the tiers, their guarantees, and the rules for using and extending them.

---

## Portability Tiers

### Tier 0: Core (Universal)

**Available:** Everywhere (std, no_std, embedded, WASM)  
**No dependencies:** Only language primitives, no external crates

**APIs:**
- Basic types: int, float, bool, string, arrays, structs
- Pure functions: arithmetic, logic, string manipulation
- Stack allocation and static allocation
- Pattern matching and control flow
- Function pointers and higher-order functions

**Example:**
```rust
pub fn add(int x, int y) -> int {
    return x + y;
}

pub fn reverse<T>(items: &array<T>) -> array<T> {
    let mut result: array<T>(items.len());
    for i in 0..items.len() {
        result[items.len() - 1 - i] = items[i];
    }
    return result;
}
```

**Guarantees:**
- Zero allocator dependency
- Runs on all targets without features
- Same performance on all backends
- No platform-specific code

---

### Tier 1: Alloc (Heap Support)

**Available:** Any target with allocator (`#[cfg(feature = "alloc")]`)  
**Dependency:** Allocator (global or injected)

**APIs:**
- Dynamic arrays (Vec-like containers)
- String construction and mutation
- Hash tables / dictionaries
- Linked structures
- Reference counting (Rc, Arc)

**Example:**
```rust
#[cfg(feature = "alloc")]
pub fn collect_results<T>(items: &array<T>) -> Vec<T> {
    let mut results = Vec::new();
    for item in items {
        results.push(item);
    }
    return results;
}
```

**Constraints:**
- Only available when compiled with `alloc` feature
- May fail if allocator runs out of memory
- Allocation is not deterministic (timing varies)
- Not suitable for hard real-time systems

**Guarantees:**
- Memory safety (no leaks, no double-free)
- Thread-safety (with proper synchronization)
- API stable across all allocator implementations

---

### Tier 2: Std (Host Only)

**Available:** Linux, Windows, macOS, WASM only  
**Dependency:** Standard library

**APIs:**
- File I/O
- Environment access
- Process control
- Standard input/output
- Networking (future)
- Time and timers
- Threading (future)

**Example:**
```rust
#[cfg(feature = "std")]
pub fn read_config(path: string) -> Result<string, string> {
    // use std file I/O
}

#[cfg(not(feature = "std"))]
pub fn read_config(path: string) -> Result<string, string> {
    return Err("file I/O not available in embedded mode");
}
```

**Constraints:**
- Only available on host targets
- Requires OS support (not suitable for bare metal)
- May depend on external system libraries
- Platform-specific behavior

**Guarantees:**
- Same API across all host platforms (with platform-specific details documented)
- Access to system resources
- Integration with OS facilities

---

### Tier 3: Platform-Specific

**Available:** Only on specific platforms  
**Dependency:** Platform-specific system libraries

**APIs:**
- MMIO registers (embedded)
- Interrupt handlers (embedded)
- System calls (Unix/Windows)
- GPU operations (future)
- Custom hardware (platform-dependent)

**Example:**
```rust
#[cfg(target = "thumbv7em-none-eabihf")]
pub mod cortex_m {
    pub fn enable_interrupts() {
        // Cortex-M specific
    }
}

#[cfg(target = "riscv32imac-unknown-none-elf")]
pub mod risc_v {
    pub fn enable_interrupts() {
        // RISC-V specific
    }
}
```

**Constraints:**
- Only usable on the target platform
- API may differ significantly between platforms
- Implementation is platform-dependent

---

## Feature Flags for Portability

### Standard Feature Set

```toml
[features]
default = ["std", "alloc"]
std = []
alloc = []
core = []  # core-only (explicit opt-out)
```

### Feature Combinations

| Features | Tier | Use Case |
|----------|------|----------|
| (none) | Core | Bare metal, extreme constraints |
| alloc | Alloc | Embedded with heap |
| std | Std | Desktop/server |
| std + alloc | Std + Alloc | Full platform |

**Example dependency with features:**
```toml
[dependencies]
my_lib = { version = "1.0" }  # gets default (std + alloc)
my_lib = { version = "1.0", default-features = false }  # core only
my_lib = { version = "1.0", default-features = false, features = ["alloc"] }  # alloc only
```

---

## Writing Portable Code

### Pattern 1: Conditional Compilation with Fallbacks

```rust
#[cfg(feature = "std")]
pub fn read_file(path: string) -> Result<string, string> {
    // Use std file I/O
}

#[cfg(not(feature = "std"))]
pub fn read_file(path: string) -> Result<string, string> {
    Err("file I/O not available in no_std".to_string())
}
```

### Pattern 2: Tier-Based Abstractions

```rust
// Tier 0: Core
pub fn hash(data: &array<byte>) -> int {
    // Pure computation, no allocation
    let mut h = 0;
    for b in data {
        h = h ^ (b as int);
    }
    return h;
}

// Tier 1: Alloc
#[cfg(feature = "alloc")]
pub fn hash_vec(data: &Vec<byte>) -> int {
    let slice = data.as_slice();  // Core function
    return hash(slice);
}

// Tier 2: Std
#[cfg(feature = "std")]
pub fn hash_file(path: string) -> Result<int, string> {
    let data = try read_file(path);  // from Tier 2
    return Ok(hash(data.as_bytes()));  // use Core
}
```

### Pattern 3: Optional Optimization with Features

```rust
pub fn process(data: &array<int>) -> int {
    #[cfg(feature = "fast_path")]
    {
        return process_optimized(data);
    }
    
    #[cfg(not(feature = "fast_path"))]
    {
        return process_generic(data);
    }
}
```

---

## Allocator Abstractions

### Global Allocator

```rust
#[cfg(feature = "alloc")]
extern "C" {
    fn malloc(size: int) -> &mut byte;
    fn free(ptr: &mut byte);
}

#[cfg(feature = "alloc")]
pub struct Allocator {
    // global allocator instance
}

#[cfg(feature = "alloc")]
impl Allocator {
    pub fn alloc(size: int) -> Option<&mut byte> {
        // platform-specific
    }
}
```

### Injected Allocator

```rust
pub struct Config<A: Allocator> {
    allocator: A,
}

pub fn with_allocator<A: Allocator>(alloc: A) -> Config<A> {
    Config { allocator: alloc }
}
```

---

## Embedded Portability

### Constraint Profile

```rust
#[cfg(target = "embedded")]
pub mod constraints {
    // Typical embedded constraints:
    // - No dynamic allocation (or very limited)
    // - No concurrency (single-threaded)
    // - Deterministic timing required
    // - Memory budget: 256 KB - 2 MB
    // - No OS (bare metal)
}
```

### Embedded-Friendly APIs

```rust
#[cfg(all(feature = "alloc", target = "embedded"))]
compile_error!("heap allocation not available in embedded mode");

pub fn embedded_safe_process(input: &array<int>) -> Result<int, string> {
    // Stack-only, no allocation
    let mut result = 0;
    for v in input {
        result += v;
    }
    return Ok(result);
}
```

---

## Testing Across Tiers

### Test Organization

```rust
#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_core_function() {
        // runs everywhere
    }
    
    #[test]
    #[cfg(feature = "alloc")]
    fn test_alloc_function() {
        // runs only with alloc
    }
    
    #[test]
    #[cfg(feature = "std")]
    fn test_std_function() {
        // runs only on host
    }
}
```

### Continuous Integration Matrix

```yaml
test_matrix:
  - feature_set: "core"      # no features
    targets: ["x86_64", "cortex-m4", "riscv32"]
  - feature_set: "alloc"     # alloc only
    targets: ["x86_64", "cortex-m4"]
  - feature_set: "std"       # std only
    targets: ["x86_64"]
  - feature_set: "std+alloc" # full
    targets: ["x86_64"]
```

---

## Documentation Requirements

### For Public APIs

**Example:**
```rust
/// Computes sum of integers in array.
/// 
/// # Availability
/// - Tier 0 (Core): Always available
/// 
/// # Example
/// ```
/// let arr = [1, 2, 3];
/// assert_eq!(sum(&arr), 6);
/// ```
pub fn sum(items: &array<int>) -> int {
    let mut total = 0;
    for item in items {
        total += item;
    }
    return total;
}

/// Reads file into string.
/// 
/// # Availability
/// - Tier 2 (Std): Requires `std` feature
/// 
/// # Errors
/// - File not found
/// - Permission denied
/// - I/O error
/// 
/// # Example
/// ```ignore
/// let content = read_file("config.txt")?;
/// ```
#[cfg(feature = "std")]
pub fn read_file(path: string) -> Result<string, string> {
    // ...
}
```

---

## Migration Path: Tier-Locked Code to Portable

### Step 1: Identify tier-specific calls

```rust
// Before: depends on std
pub fn process(path: string) -> int {
    let data = read_file(path);  // std only
    return compute(data);
}
```

### Step 2: Extract core logic

```rust
pub fn compute(data: string) -> int {
    // Tier 0: Core logic, no I/O
    let mut count = 0;
    for ch in data {
        count += 1;
    }
    return count;
}
```

### Step 3: Make I/O conditional

```rust
#[cfg(feature = "std")]
pub fn process_file(path: string) -> Result<int, string> {
    let data = try read_file(path);
    return Ok(compute(data));  // use core function
}

pub fn process_data(data: string) -> int {
    return compute(data);  // works everywhere
}
```

---

## Best Practices

### 1. Write Core First, Extend Later

```rust
// Good: core function first
pub fn find_max(items: &array<int>) -> int {
    let mut max = items[0];
    for item in items {
        if item > max { max = item; }
    }
    return max;
}

// Extended: higher-tier wrapper
#[cfg(feature = "alloc")]
pub fn find_max_vec(items: &Vec<int>) -> int {
    find_max(items.as_slice())
}
```

### 2. Document Tier Requirements

```rust
/// Core algorithm: O(n) sum.
/// Availability: Tier 0 (Core)
pub fn sum_core(items: &array<int>) -> int { ... }

/// Heap-backed result collection.
/// Availability: Tier 1 (Alloc)
#[cfg(feature = "alloc")]
pub fn collect_all(items: &array<int>) -> Vec<int> { ... }
```

### 3. Test Each Tier

- Core: Test without features
- Alloc: Test with `alloc` only
- Std: Test with `std` only
- Full: Test with both

### 4. Minimize Feature Usage

Fewer features = more portable code.

```rust
// Prefer: fewer dependencies
pub fn swap<T>(a: &mut T, b: &mut T) {
    // no features needed
}

// Avoid: gratuitous features
pub fn swap_with_logging<T>(a: &mut T, b: &mut T) {
    #[cfg(feature = "std")]
    eprintln!("swapping");
}
```

---

## Roadmap

### v0.4: Tier Definition and Tooling

- [ ] Feature flag conventions
- [ ] Portability checking in compiler
- [ ] CI matrix generation

### v0.5: Standard Library Expansion

- [ ] Complete Tier 1 (Alloc) APIs
- [ ] Stable Tier 2 (Std) APIs
- [ ] Allocator traits and implementations

### v0.6+: Advanced Portability

- [ ] WASM tier support
- [ ] Async portability (Future traits)
- [ ] No_std error handling improvements
- [ ] Embedded-specific tooling (linker scripts, memory layouts)

---

## Summary Table

| Tier | Availability | Allocation | I/O | Concurrency | Use Case |
|------|-------------|-----------|-----|-------------|----------|
| 0: Core | Universal | Stack/static | No | No | All platforms |
| 1: Alloc | With feature | Heap | No | No | Embedded + host |
| 2: Std | Host only | Heap | Yes | No (future) | Desktop/server |
| 3: Platform | Platform-specific | Platform | Yes | Platform | Native code |

---

## References

- **RES-3507:** Design a production-grade standard library portability model
- **RES-3502:** Module and package system design
- **MEMORY_MODEL.md:** Allocation tiers and constraints
- **MODULE_SYSTEM.md:** Package feature flags
