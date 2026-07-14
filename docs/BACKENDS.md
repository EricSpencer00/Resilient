---
title: Backend Architecture
parent: Language Reference
nav_order: 5
permalink: /backends
---

# Resilient Backend Architecture

## Overview

Resilient supports multiple execution backends for different use cases. This document defines the architecture contract, feature support matrix, and selection guidance for each backend.

> **Reconcile-to-reality note (RES-3506.1, 2026-07):** an earlier draft
> of this document's feature matrix overstated JIT (and, in a few
> places, VM) support for Effects, MMIO, and Concurrency. Corrections
> below are grounded directly in `resilient/src/jit_backend.rs`,
> `resilient/src/vm.rs`, and `resilient/src/compiler.rs` — every "✅
> Stable" or "⚠️ Backend-Limited" claim in the tables cites the source
> evidence for it inline or in a footnote. **The interpreter (tree-walker)
> is the canonical oracle**: `--vm` and `--jit` are alternate execution
> strategies for the *same* AST, and any observable difference from the
> interpreter's value typing, error classes, or operator semantics on a
> Stable-tier program is a backend bug, not an accepted variance
> (`resilient/tests/it/differential.rs` is the parity harness that
> enforces this across all three backends).

| Backend | Purpose | Stability | Performance | Memory |
|---------|---------|-----------|-------------|--------|
| **Interpreter** | Development, debugging, prototyping | Stable | Slow (1000x) | High |
| **VM** | Bytecode execution on desktop hosts today; embedded is the design target¹ | Stable | Medium (10-100x) | Low |
| **JIT** | Production systems, performance-critical | Backend-Limited | Fast (1-5x native) | Medium |
| **Verifier** | Safety proofs, formal verification | Experimental | N/A (static) | N/A |

¹ See the VM section's "Target Platforms" correction below: `resilient/src/vm.rs`
runs only on desktop hosts (`x86_64-unknown-linux-gnu` and equivalent) today.
No CI job cross-compiles `resilient/` (the crate containing `vm.rs`) to any
embedded target — only the separate `resilient-runtime`/
`resilient-runtime-cortex-m-demo` crates get embedded builds.

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
| Generics | ✅ Stable | `Value` is dynamically typed, so a generic fn body runs unchanged per instantiation (no separate specialization step needed for the interpreter) |
| Effects | ❌ Not Supported | **Correction:** no effect-annotation syntax (`! IO`) exists in the lexer/parser at all — this is not "parsed but unenforced," there is nothing to parse. See `docs/TYPE_SYSTEM_ROADMAP.md` Phase 2. |
| Memory tiers (Stack/Static/Heap/MMIO) | ✅ Stable | All supported; MMIO via the `volatile_read_*`/`volatile_write_*` builtins (`resilient/src/volatile.rs`), dispatched through the same `BUILTINS` table as every other builtin |
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
- Pre-compilation pass (`compiler.rs` lowers `Node` → `Chunk`)
- **Stack-based**, not register-based — corrected below
- Runs on desktop targets today (`x86_64-unknown-linux-gnu`); the
  embedded cross-compile story lives in `resilient-runtime`, a
  separate `#![no_std]` crate (see `docs/MEMORY_MODEL.md`), not in
  `vm.rs` itself

**Correction:** an earlier draft of this section described the VM as
"register-based (16 registers)". `resilient/src/bytecode.rs`'s doc
comment says otherwise: *"The VM is stack-based: most ops pop their
arguments and push their result."* There is no register file in `Op`
or `VmState`.

### Instruction Set

Real opcodes from the `Op` enum in `resilient/src/bytecode.rs`
(abbreviated — the full enum also carries closures, upvalues, and
struct/enum construction ops added by later tickets):

- **Arithmetic/Logic:** `Add`, `Sub`, `Mul`, `Div`, `Mod`, `Neg`, `Not`
- **Comparison:** `Eq`, `Neq`, `Lt`, `Le`, `Gt`, `Ge` (no generic `cmp` —
  each comparison is its own opcode)
- **Locals:** `Const(idx)`, `LoadLocal(idx)`, `StoreLocal(idx)`,
  `IncLocal(idx)` (peephole-fused increment)
- **Control flow:** `Jump(offset)`, `JumpIfFalse(offset)`,
  `JumpIfTrue(offset)`, `Call(idx)`, `ReturnFromCall`, `Return` (halts
  the whole VM; distinct from `ReturnFromCall`, which returns from one
  frame)
- **Closures:** `MakeClosure`, `LoadUpvalue`, `StoreUpvalue`
- **Builtins:** a generic call-builtin opcode (`h_call_builtin` in
  `vm.rs`) that resolves the callee name through the same
  `crate::lookup_builtin` registry the interpreter uses — this is how
  MMIO (`volatile_read_*`/`volatile_write_*`) and `spawn`/`send`
  builtins reach the VM, with no dedicated opcode of their own

**Correction:** an earlier draft of this section listed `and`, `or`,
`alloc`, `free`, `read`, `write`, `mmio_read`, `mmio_write` opcodes and
an `enter_effect`/`exit_effect` "Effects" category. None of these exist
— `grep -n "Op::Read\|Op::Write\|mmio\|[Ee]ffect" resilient/src/vm.rs
resilient/src/compiler.rs resilient/src/bytecode.rs` returns no opcode
or category by those names. Short-circuit `and`/`or` compile to
`Jump`/`JumpIfFalse` sequences rather than dedicated opcodes; there is
no heap `alloc`/`free` opcode (heap-backed values are host-allocated
`Value` variants, not manually managed VM memory); there is no
effect-tracking mechanism in the bytecode at all (see the Effects row
below); and MMIO goes through the generic builtin-call path, not a
dedicated instruction.

### Feature Support

| Feature | Status | Notes |
|---------|--------|-------|
| Basic types | ✅ Stable | Full support |
| Functions | ✅ Stable | Tail-call optimization |
| Arrays | ✅ Stable | Bounds checked |
| Structs | ✅ Stable | Stack layout optimized |
| Pattern matching | ✅ Stable | Compiled to jump tables |
| Generics | ✅ Stable | Monomorphization at compile time (`monomorph::lower` runs before `compiler::compile`) |
| Effects | ❌ Not Supported | No effect-annotation syntax exists (same as Interpreter row above); no bytecode representation for it either |
| Stack allocation | ✅ Stable | Frame-relative `LoadLocal`/`StoreLocal` |
| Static allocation | ✅ Stable | `Const` pool entries |
| Heap allocation | ⚠️ Backend-Limited | Requires allocator configuration |
| MMIO access | ✅ Stable | Via the generic builtin-call path (`volatile_read_*`/`volatile_write_*`), not a dedicated opcode — see Instruction Set above |
| Concurrency | ❌ Not Supported | No task scheduler; `spawn`/`send` builtins exist and are reachable via the generic builtin-call path (`resilient/src/actor_runtime.rs`), but the scheduler is a `thread_local!` cooperative mailbox model, not true parallelism |
| JIT compilation | ❌ N/A | Different backend |
| Verification | ❌ N/A | Different backend |

### Target Platforms

**Correction:** an earlier draft of this section listed
`thumbv7em-none-eabihf`, `thumbv6m-none-eabi`, and
`riscv32imac-unknown-none-elf` as VM target platforms, implying
`resilient/src/vm.rs` (this backend) cross-compiles to and runs on
embedded hardware today. It does not: `resilient/Cargo.toml` has no
embedded-target configuration, and `.github/workflows/embedded.yml`'s
cross-compile jobs build only `resilient-runtime-cortex-m-demo` and
`resilient-runtime` — the separate `#![no_std]` runtime-types crate
documented in `docs/MEMORY_MODEL.md` — never `resilient/` itself. There
is currently **no `.rz` source → embedded-binary pipeline**; `vm.rs`
runs on the same host targets as the interpreter and JIT.

- **x86-64 / ARM64 (desktop, actual today):** `x86_64-unknown-linux-gnu`
  and equivalent host targets, via `rz --vm`
- **Cortex-M / RISC-V (design target, not yet wired):** the intent is
  for this backend to become the embedded execution strategy once a
  `.rz`-to-embedded pipeline exists; until then, embedded deployments
  use `resilient-runtime`'s Rust API directly (see `docs/MEMORY_MODEL.md`),
  not the VM bytecode format described here

### Memory Model

The table below describes the **design target** for embedded
deployment, not `vm.rs`'s current desktop-only behavior (see the
Target Platforms correction above).

| Tier | Strategy | Notes |
|------|----------|-------|
| Stack | SRAM base + offset | Design target — not applicable until the embedded pipeline exists |
| Static | Flash base (with BSS) | Design target — not applicable until the embedded pipeline exists |
| Heap | Allocator instance | Optional, configurable |
| MMIO | Hardware address | Volatile reads/writes; today this only executes via the host-side `volatile_read_*`/`volatile_write_*` builtins (test buffers, not real hardware — see `resilient/src/volatile.rs`'s own doc comment: "No real MMIO addresses are touched") |

### Conformance Rules

1. All Stable features must work identically across all target platforms the backend actually ships on today (desktop hosts; embedded is a design target, not yet a shipped target — see above)
2. The ≤ 64 KiB `.text` size budget applies to `resilient-runtime-cortex-m-demo`, not to this VM backend (which has no embedded build at all today)
3. Stack overflow must be detectable (guard pages where available)
4. MMIO accesses must preserve volatile semantics
5. No panics in no_std runtime; use `Result` instead — this rule applies to `resilient-runtime`, not to `resilient/src/vm.rs`, which is a `std` crate

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

**Correction:** an earlier draft of this section listed seven passes.
Only two are backed by Resilient-authored code in `jit_backend.rs`; the
rest either don't exist as Resilient passes or are properties of
Cranelift (the underlying codegen library) rather than something
`jit_backend.rs` implements or orchestrates:

1. **Monomorphization (real):** `monomorph::lower` specializes generic
   functions to concrete types before the JIT sees the AST (RES-405).
2. **Trivial-leaf inlining (real, narrower than "Inlining" implies):**
   `is_trivial_leaf`/`try_lower_inline_call` (RES-175) inline a callee
   only when its body is ≤ `TRIVIAL_LEAF_MAX_NODES` (8) nodes, contains
   no calls/loops/match/array-literal/index-expression (the
   `has_disqualifying_construct` predicate), and isn't self-recursive.
   This is a narrow leaf-inlining heuristic, not general aggressive
   inlining.
3. **Tail-call optimization (real, not listed in the original draft):**
   a `ReturnStatement` whose value is a direct call to the
   currently-compiling function (matching arity) lowers to a jump back
   into the function body instead of a call+return (RES-168).
4. Dead code elimination, constant folding, strength reduction, loop
   unrolling, and peephole optimization are **not implemented as
   Resilient-authored passes** — `grep -in
   "loop.unroll\|strength.reduc\|peephole\|dead.code\|constant.fold"
   resilient/src/jit_backend.rs` finds nothing besides an unrelated
   `#![allow(dead_code)]` lint attribute. Cranelift, the codegen crate
   Resilient's JIT lowers into, does its own internal optimization at
   the IR level, but that is a property of the Cranelift dependency,
   not a documented Resilient contract.

### Feature Support

| Feature | Status | Notes |
|---------|--------|-------|
| Basic types | ✅ Stable | Full support |
| Functions | ✅ Stable | Aggressive inlining |
| Arrays | ✅ Stable | Bounds check elimination |
| Structs | ✅ Stable | Field access optimized |
| Pattern matching | ✅ Stable | Jump table optimization |
| Generics | ✅ Stable | `monomorph::lower` runs before JIT compilation (same mechanism as the VM row above) |
| Effects | ❌ Not Supported | **Correction:** no effect-annotation syntax exists anywhere in the language (see Interpreter/VM rows above); "affects optimization strategy" in an earlier draft was describing a feature that doesn't exist |
| Stack allocation | ✅ Stable | Cranelift `Variable`-backed locals |
| Static allocation | ✅ Stable | Constant pool (`iconst`, `f64const`) |
| Heap allocation | ⚠️ Backend-Limited | Array literals above a small size limit are rejected (`"array literal too large for JIT"`); struct/map literals lower via runtime shim calls, not inline allocation |
| MMIO access | ❌ Not Supported | **Correction:** `lookup_jit_builtin` only allowlists `abs`, `len`, `max`, `min` (plus `println`/`print`/`to_string` special-cased separately) — `grep -n "volatile" resilient/src/jit_backend.rs` returns nothing. `volatile_read_*`/`volatile_write_*` are not reachable from JIT-compiled code; a program using them falls back to the interpreter or errors with `JitError::Unsupported`. |
| Concurrency | ❌ Not Supported | **Correction:** no "work-stealing scheduler" exists — `grep -in "actor\|spawn\|thread.?pool\|work.steal" resilient/src/jit_backend.rs` finds nothing beyond the Rust-standard-library `Mutex`/thread usage internal to the JIT compiler's own test harness. `spawn`/`send` (the actor-runtime builtins) are not in the JIT builtin allowlist. |
| Verification | ❌ N/A | Different backend |

### Target Platforms

- **x86-64:** x86_64-unknown-linux-gnu, x86_64-pc-windows-msvc
- **ARM64:** aarch64-unknown-linux-gnu
- **Embedded:** not supported — same gap noted in the VM section above; no `.rz`-to-embedded pipeline exists yet for any backend

### Performance Targets

**Correction:** an earlier draft cited a "< 50 MB for typical embedded
applications" memory target for a backend that (per the row above)
doesn't run on embedded targets at all. Kept the startup/runtime targets,
which are plausible desktop-JIT goals; dropped the embedded-memory claim
as unbacked by any test or CI gate found in `resilient/`.

- Startup: < 100ms for typical programs
- Runtime: Within 1.5x of hand-written C for compute-heavy workloads

### Conformance Rules

1. Must produce output identical to interpreter (within floating-point precision) — enforced by `resilient/tests/it/differential.rs`
2. Unsupported AST shapes must fail cleanly via `JitError::Unsupported`, never panic — this is the actual contract (`has_disqualifying_construct` gates only the trivial-leaf inliner, not general JIT support; the real support boundary is whatever `lower_expr`/`lower_stmt` handles before falling through to the `node_kind`-tagged `JitError::Unsupported` catch-all)
3. Stack allocation must not exceed platform limits

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
| Effects | ❌ | ❌ | ❌ | N/A |
| Stack allocation | ✅ | ✅ | ✅ | ⚠️ |
| Static allocation | ✅ | ✅ | ✅ | ✅ |
| Heap allocation | ✅ | ⚠️ | ✅ | ⚠️ |
| MMIO access | ✅ | ✅ | ❌ | ❌ |
| Concurrency | ❌ | ❌ | ❌ | ❌ |

**Corrections from an earlier draft:** the Effects row was `⚠️ | ✅ | ✅
| ⚠️` — corrected to all-`❌` (N/A for the Verifier, which has no effect
system to verify either) because no effect-annotation syntax exists in
the language at all yet, on any backend (see the per-backend sections
above and `docs/TYPE_SYSTEM_ROADMAP.md` Phase 2). The MMIO row's JIT
cell was `⚠️`, corrected to `❌` (`lookup_jit_builtin` doesn't allowlist
the `volatile_*` builtins). The Concurrency row's JIT cell was `⚠️`
("Work-stealing scheduler"), corrected to `❌` — no such scheduler, or
any actor/spawn support, exists in `jit_backend.rs`.

---

## Backend Selection Guide

### Choose Interpreter If:

- **Developing:** Tight debug loop
- **Prototyping:** Quick iteration
- **Learning:** Understanding semantics
- **Testing:** Validating correctness first

### Choose VM If:

- **Production (non-critical), on desktop hosts today:** Balanced performance
- **Cross-platform (desktop):** Consistent behavior needed
- **Deployment:** Pre-compiled, deterministic
- **Not yet for:** embedded systems — there is no `.rz`-to-embedded
  pipeline today (see the VM section's Target Platforms correction);
  embedded deployments currently mean writing directly against
  `resilient-runtime`'s Rust API, not compiling `.rz` source

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

This is illustrative CI shape, not a literal copy of a workflow file.
**Correction:** an earlier draft listed `thumbv7em-none-eabihf` /
`riscv32imac-unknown-none-elf` as VM platforms — those targets are
built in `.github/workflows/embedded.yml`, but for the
`resilient-runtime`/`resilient-runtime-cortex-m-demo` crates, not for
`resilient/` (which is what actually runs `--vm`). Corrected to match
what CI actually builds today:

```yaml
backends:
  - interpreter
    platforms: [x86_64-unknown-linux-gnu]
  - vm
    platforms: [x86_64-unknown-linux-gnu]   # desktop only — see Target Platforms note above
  - jit
    platforms: [x86_64-unknown-linux-gnu, aarch64-unknown-linux-gnu]
    features: [jit]
  - verifier
    features: [z3]

# Separate embedded cross-compile gate (unrelated crate, not a
# `resilient` execution backend):
embedded_runtime_crates:
  - resilient-runtime-cortex-m-demo: [thumbv7em-none-eabihf]
  - resilient-runtime: [thumbv6m-none-eabi, riscv32imac-unknown-none-elf]
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
