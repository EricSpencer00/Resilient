---
title: Backend Architecture
parent: Language Reference
nav_order: 5
permalink: /backends
---

# Resilient Backend Architecture

## Overview

Resilient supports multiple execution backends for different use cases. This document defines the architecture contract, feature support matrix, and selection guidance for each backend.

| Backend | Purpose | Stability | Performance | Memory |
|---------|---------|-----------|-------------|--------|
| **Interpreter** | Development, debugging, prototyping | Stable | Slow (1000x) | High |
| **VM** | Embedded systems, resource-constrained | Stable | Medium (10-100x) | Low |
| **JIT** | Production systems, performance-critical | Backend-Limited | Fast (1-5x native) | Medium |
| **Verifier** | Safety proofs, formal verification | Experimental | N/A (static) | N/A |

---

## Backend: Interpreter

### Architecture

The interpreter is a tree-walking evaluator that directly executes the AST:

```
Source Code → Parser → AST → Interpreter → Output
```

**Characteristics:**
- Direct AST traversal
- No compilation step
- Immediate feedback
- Full source-level debugging

### Feature Support

| Feature | Status | Notes |
|---------|--------|-------|
| Basic types (int, string, bool) | ✅ Stable | Full support |
| Functions | ✅ Stable | Recursion supported |
| Arrays | ✅ Stable | Dynamic sizing |
| Structs | ✅ Stable | Named fields |
| Pattern matching | ✅ Stable | Exhaustiveness checked |
| Generics | ✅ Stable | Monomorphization at interpret time |
| Effects | ⚠️ Backend-Limited | Parsed but not enforced |
| Memory tiers (Stack/Static/Heap/MMIO) | ✅ Stable | All supported |
| Concurrency | ❌ Not Supported | Single-threaded only |
| JIT inline hints | ❌ N/A | Not applicable |
| SMT verification | ❌ N/A | No static analysis |

### Use Cases

- **Development:** Rapid prototyping without compile times
- **Debugging:** Step through code execution
- **Testing:** Quick validation of logic
- **Education:** Understanding language semantics

### Conformance Rules

1. Must accept all valid Stable-tier code
2. Must produce identical results to other backends
3. Memory safety violations must be runtime errors, not panics
4. Diagnostics must match compiler error messages

---

## Backend: VM

### Architecture

The VM is a bytecode interpreter with ahead-of-time compilation:

```
Source Code → Parser → Type Check → Bytecode → VM → Output
```

**Characteristics:**
- Bytecode-based execution
- Pre-compilation pass
- Register-based VM (16 registers)
- Optimized for embedded targets

### Instruction Set

Core instruction categories:
- **Arithmetic:** `add`, `sub`, `mul`, `div`, `mod`
- **Logic:** `and`, `or`, `not`, `cmp`
- **Memory:** `load`, `store`, `alloc`, `free`
- **Control:** `jump`, `branch`, `call`, `return`
- **I/O:** `read`, `write`, `mmio_read`, `mmio_write`
- **Effects:** `enter_effect`, `exit_effect` (metadata, not enforced)

### Feature Support

| Feature | Status | Notes |
|---------|--------|-------|
| Basic types | ✅ Stable | Full support |
| Functions | ✅ Stable | Tail-call optimization |
| Arrays | ✅ Stable | Bounds checked |
| Structs | ✅ Stable | Stack layout optimized |
| Pattern matching | ✅ Stable | Compiled to jump tables |
| Generics | ✅ Stable | Monomorphization at compile time |
| Effects | ✅ Stable | Tracked in bytecode |
| Stack allocation | ✅ Stable | SRAM-based |
| Static allocation | ✅ Stable | Flash or ROM |
| Heap allocation | ⚠️ Backend-Limited | Requires allocator configuration |
| MMIO access | ✅ Stable | Hardware registers supported |
| Concurrency | ❌ Not Supported | No task scheduler |
| JIT compilation | ❌ N/A | Different backend |
| Verification | ❌ N/A | Different backend |

### Target Platforms

- **Cortex-M (ARM Embedded):** thumbv7em-none-eabihf, thumbv6m-none-eabi
- **RISC-V:** riscv32imac-unknown-none-elf
- **x86-64 (simulation):** x86_64-unknown-linux-gnu (testing only)

### Memory Model

| Tier | Strategy | Notes |
|------|----------|-------|
| Stack | SRAM base + offset | Known at link time |
| Static | Flash base (with BSS) | Zero-initialized |
| Heap | Allocator instance | Optional, configurable |
| MMIO | Hardware address | Volatile reads/writes |

### Conformance Rules

1. All Stable features must work identically across all target platforms
2. Embedded targets must run with ≤ 64 KiB `.text` section (size budget)
3. Stack overflow must be detectable (guard pages where available)
4. MMIO accesses must preserve volatile semantics
5. No panics in no_std runtime; use `Result` instead

---

## Backend: JIT

### Architecture

The JIT backend compiles bytecode to native machine code at load time:

```
Source Code → Parser → Type Check → Bytecode → JIT → Native Code → CPU → Output
```

**Characteristics:**
- Just-in-time compilation to native code
- Aggressive optimization passes
- Low latency execution
- Target-specific instruction selection

### Optimization Passes

1. **Monomorphization:** Specialize generics to concrete types
2. **Inlining:** Inline small functions
3. **Dead code elimination:** Remove unreachable code
4. **Constant folding:** Evaluate constants at compile time
5. **Strength reduction:** Replace expensive ops with cheaper ones
6. **Loop unrolling:** Unroll hot loops (with heuristics)
7. **Peephole:** Local instruction optimization

### Feature Support

| Feature | Status | Notes |
|---------|--------|-------|
| Basic types | ✅ Stable | Full support |
| Functions | ✅ Stable | Aggressive inlining |
| Arrays | ✅ Stable | Bounds check elimination |
| Structs | ✅ Stable | Field access optimized |
| Pattern matching | ✅ Stable | Jump table optimization |
| Generics | ✅ Stable | Full specialization |
| Effects | ✅ Stable | Affects optimization strategy |
| Stack allocation | ✅ Stable | Stack frame optimization |
| Static allocation | ✅ Stable | RIP-relative addressing |
| Heap allocation | ✅ Stable | Inline allocations |
| MMIO access | ⚠️ Backend-Limited | Platform-dependent |
| Concurrency | ⚠️ Backend-Limited | Work-stealing scheduler |
| Verification | ❌ N/A | Different backend |

### Target Platforms

- **x86-64:** x86_64-unknown-linux-gnu, x86_64-pc-windows-msvc
- **ARM64:** aarch64-unknown-linux-gnu
- **Embedded (future):** Integration with embedded JIT framework

### Performance Targets

- Startup: < 100ms for typical programs
- Runtime: Within 1.5x of hand-written C for compute-heavy workloads
- Memory: < 50 MB for typical embedded applications

### Conformance Rules

1. Must produce output identical to interpreter (within floating-point precision)
2. Optimization passes must be semantics-preserving
3. Effect tracking must be accurate (no dead effect code elimination)
4. Must respect platform-specific MMIO constraints
5. Stack allocation must not exceed platform limits

---

## Backend: Verifier (Z3-based)

### Architecture

The verifier uses SMT solvers for formal verification:

```
Source Code → Parser → Type Check → Constraints → Z3 Solver → Proof/Counterexample
```

**Characteristics:**
- Static analysis (no execution)
- SMT-LIB2 constraint generation
- Automated theorem proving
- Generates proofs or counterexamples

### Verification Capabilities

| Capability | Status | Notes |
|------------|--------|-------|
| Type safety | ✅ Stable | Leverages type system |
| Memory safety | ✅ Stable | Lifetime + bounds checking |
| Integer overflow | ✅ Stable | Signed/unsigned analysis |
| Deadlock detection | ⚠️ Experimental | Concurrency model incomplete |
| Liveness proofs | ⚠️ Experimental | Requires effect boundaries |
| Custom assertions | ⚠️ Backend-Limited | Requires Z3 integration |

### Feature Support

| Feature | Status | Notes |
|---------|--------|-------|
| Basic types | ✅ Stable | Bitvector logic |
| Integer ranges | ✅ Stable | SMT-LIB support |
| Arrays | ✅ Stable | Array theory |
| Structs | ✅ Stable | Flattened to fields |
| Functions | ⚠️ Backend-Limited | Requires unrolling |
| Recursion | ⚠️ Experimental | Bounded unrolling |
| Generics | ⚠️ Backend-Limited | Monomorphized then verified |
| Floating-point | ⚠️ Experimental | IEEE-754 theory (incomplete) |
| Concurrency | ❌ Not Supported | Sequential assumptions |
| Optimization | ❌ N/A | Static analysis only |

### Usage Model

```rust
#[verify]  // Request verification for this function
fn safe_add(x: int, y: int) -> int {
    // Verifier will prove no overflow if constraints hold
    return x + y;
}

#[verify(bound = "x <= 1000000")]
fn bounded_add(x: int, y: int) -> int {
    return x + y;
}
```

### Conformance Rules

1. All assertions must be satisfiable within reasonable time (< 10s per function)
2. Proofs must be reproducible (deterministic solver state)
3. Counterexamples must be minimal and debuggable
4. Must not claim to verify unverifiable code

---

## Feature Matrix by Backend

| Feature | Interpreter | VM | JIT | Verifier |
|---------|-------------|----|----|----------|
| Tier: Stable | ✅ | ✅ | ✅ | ❌ |
| Tier: Backend-Limited | ⚠️ | ⚠️ | ⚠️ | ✅ |
| Tier: Experimental | ✅ | ✅ | ✅ | ✅ |
| | | | | |
| Basic types | ✅ | ✅ | ✅ | ✅ |
| Functions | ✅ | ✅ | ✅ | ⚠️ |
| Generics | ✅ | ✅ | ✅ | ⚠️ |
| Structs | ✅ | ✅ | ✅ | ✅ |
| Pattern matching | ✅ | ✅ | ✅ | ⚠️ |
| Effects | ⚠️ | ✅ | ✅ | ⚠️ |
| Stack allocation | ✅ | ✅ | ✅ | ⚠️ |
| Static allocation | ✅ | ✅ | ✅ | ✅ |
| Heap allocation | ✅ | ⚠️ | ✅ | ⚠️ |
| MMIO access | ✅ | ✅ | ⚠️ | ❌ |
| Concurrency | ❌ | ❌ | ⚠️ | ❌ |

---

## Backend Selection Guide

### Choose Interpreter If:

- **Developing:** Tight debug loop
- **Prototyping:** Quick iteration
- **Learning:** Understanding semantics
- **Testing:** Validating correctness first

### Choose VM If:

- **Embedded systems:** Size/resource constraints
- **Production (non-critical):** Balanced performance
- **Cross-platform:** Consistent behavior needed
- **Deployment:** Pre-compiled, deterministic

### Choose JIT If:

- **Performance-critical:** Native-speed execution needed
- **Server workloads:** High throughput required
- **Desktop apps:** Low latency expected
- **Optimization:** Aggressive specialization beneficial

### Choose Verifier If:

- **Safety-critical:** Formal proofs required
- **Security:** Exhaustive analysis needed
- **Compliance:** Certification demands
- **Research:** Exploring verification techniques

---

## Implementation Rules

### Backend Invariants

1. **Identical semantics:** All backends produce identical results on Stable code
2. **Error consistency:** Runtime errors have the same origin and message
3. **Determinism:** No non-deterministic behavior (for reproducibility)
4. **Type safety:** No type violations possible
5. **Memory safety:** No use-after-free, double-free, or data races

### Adding a New Backend

1. Implement minimal feature set (basic types, functions, control flow)
2. Pass interpreter conformance test suite
3. Document feature support matrix
4. Add platform-specific tests
5. Graduate from Experimental to Backend-Limited
6. Eventually graduate to Stable if replaces existing backend

### Feature Availability Rules

| Tier | Must support on | Can selectively support | Cannot support |
|------|-----------------|------------------------|-----------------|
| Stable | All backends | N/A | N/A |
| Backend-Limited | Specified backends | Explicitly documented | Non-specified backends |
| Experimental | At least one backend | Yes | Not required |

---

## Platform-Specific Notes

### Cortex-M (ARM Embedded)

- **Constraint:** Typically ≤ 256 KB Flash, ≤ 64 KB RAM
- **Typical budget:** 64 KB `.text`, 8 KB stack, 8 KB static data
- **Optimization:** Aggressive code size optimization (-Os)
- **Features:** Full MMIO support for STM32, nRF, LPC families

### RISC-V

- **Constraint:** Typically ≤ 4 MB Flash, variable RAM
- **Typical budget:** 256 KB `.text`, 16 KB stack, 64 KB static data
- **Optimization:** RV32IMC ISA subset
- **Features:** Full MMIO support for RISC-V interrupt controller

### x86-64 (Linux/Windows)

- **Constraint:** Modern systems (2+ GB RAM typical)
- **Optimization:** Aggressive optimization, parallelism possible
- **Features:** Full concurrency support, system calls available
- **Note:** Primarily for development/testing, not deployment

---

## CI/CD Integration

### Build Matrix

```yaml
backends:
  - interpreter
  - vm
    platforms: [thumbv7em-none-eabihf, riscv32imac-unknown-none-elf]
  - jit
    platforms: [x86_64-unknown-linux-gnu, aarch64-unknown-linux-gnu]
  - verifier
    features: [z3]
```

### Test Requirements

| Backend | Minimum tests | Performance gate |
|---------|---|---|
| Interpreter | 100+ integration tests | < 5s per test |
| VM | 100+ + 3 platform targets | < 100ms startup |
| JIT | 100+ + 2 platform targets | < 2s startup + perf < 1.5x native |
| Verifier | 50+ + solver timeouts | < 10s per verification |

---

## References

- **RES-3506:** Define the backend architecture contract
- **RES-3502:** Design a real module and package system
- **LANGUAGE.md:** Feature tier classification framework
- **MEMORY_MODEL.md:** Memory safety model across tiers
