# Resilient Verification Surface Unification

## Overview

Resilient provides multiple verification backends: contracts, Z3 (SMT solver), TLA+, Lean, and Stateright. This document unifies these into one coherent user-facing verification model, explaining what each backend does, when to use it, and how they interact.

---

## Core Vision

**One verification story, many backends.**

A user should be able to write assertions once and have the Resilient ecosystem automatically:
1. Check them at compile time (if feasible with Z3)
2. Check them at runtime (if verification fails)
3. Prove them formally (if verification succeeds)
4. Model-check them for distributed systems (via Stateright)

---

## Verification Backends

### Backend 1: Compile-Time Contracts (Stable)

**Purpose:** Catch errors before runtime via static analysis.

**What it does:**
- Validates `#[requires]` and `#[ensures]` preconditions/postconditions
- Compiler rejects code that violates contracts
- Zero runtime overhead

**Example:**

```rust
#[requires(x >= 0)]
#[requires(y >= 0)]
#[ensures(return >= x and return >= y)]
fn max(int x, int y) -> int {
    if x > y { return x; }
    return y;
}

// Compiler rejects:
max(-1, 5);        // error: violates precondition
max(1, 5);         // ok: satisfies requires and ensures
```

**When to use:** Every function should have contracts. Enables compile-time validation.

**Guarantee:** If compilation succeeds, preconditions are satisfied at call sites.

---

### Backend 2: Z3 SMT Solver (Experimental → Stable in v0.5)

**Purpose:** Formal verification of contracts and assertions using automated theorem proving.

**What it does:**
- Converts Resilient code to SMT-LIB2 constraints
- Sends constraints to Z3 solver
- Z3 proves or refutes the assertion
- Returns proof or counterexample

**Example:**

```rust
#[verify]  // Request Z3 verification
fn safe_add(int x, int y) -> int {
    return x + y;
}
```

**Z3 output:** "Verified: for all x, y in [INT_MIN, INT_MAX], x + y does not overflow"

**When to use:**
- Critical arithmetic (safety-critical systems)
- Complex predicates with many branches
- When you need a proof of correctness

**Limitations:**
- Works best for decidable logic (arithmetic, bit operations)
- May timeout on very complex formulas
- Floating-point support incomplete

**Integration:**

```rust
#[cfg(feature = "z3")]
#[verify]
fn critical_computation(int x, int y) -> int {
    // Z3 verifies this in -O build
    return x * y / (x + 1);
}
```

---

### Backend 3: Runtime Assertion Checking (Stable)

**Purpose:** Validate contracts during testing and development.

**What it does:**
- Evaluates `#[requires]` / `#[ensures]` at runtime
- Panics if assertion fails
- Useful for test-driven development

**Example:**

```rust
#[test]
fn test_max_preconditions() {
    // These will panic at runtime if contracts fail
    assert_eq!(max(5, 3), 5);     // satisfies contract
    // max(-1, 5);                 // would panic: precondition failed
}
```

**When to use:**
- Testing before committing
- Development builds (unoptimized)
- Validation of Z3 proofs

**Performance:** Disabled in release builds (unless explicitly enabled)

---

### Backend 4: Failure Recovery (live {} blocks)

**Purpose:** Recover from transient faults without aborting.

**What it does:**
- Specifies expected failure modes
- Provides recovery code
- Automatic state rollback on fault

**Example:**

```rust
live {
    sensor_reading = read_sensor();
    if sensor_reading < MIN || sensor_reading > MAX {
        fault "sensor out of range";
    }
} recover {
    sensor_reading = default_reading;
    log_fault("sensor failure");
}

process(sensor_reading);  // Guaranteed valid after recover
```

**When to use:**
- Handling transient hardware faults
- Network timeouts with fallback
- Data validation with default values

**Guarantee:** After recovery block, state is valid per contract.

---

### Backend 5: TLA+ Model Checking (Experimental)

**Purpose:** Verify distributed systems and concurrent algorithms.

**What it does:**
- Translates Resilient code to TLA+
- TLC model checker explores all possible interleavings
- Finds deadlocks, race conditions, safety violations

**Example:**

```rust
#[model_check]
fn test_two_phase_commit() {
    // TLA+ verifies that all interleavings
    // lead to consistent state
}
```

**When to use:**
- Consensus protocols
- Multi-actor systems
- Distributed algorithms

**Guarantee:** If TLA+ verification succeeds, no interleavings violate the invariant.

**Status:** Experimental (v0.4+), requires `#[cfg(feature = "tla")]`

---

### Backend 6: Stateright Simulation Testing (Experimental → Backend-Limited in v0.6)

**Purpose:** Test concurrent systems with exhaustive state exploration.

**What it does:**
- Simulates all possible thread interleavings
- Detects race conditions, deadlocks, livelocks
- Property-based testing for actor systems

**Example:**

```rust
#[stateright_test]
fn test_mutex_fairness() {
    // Explore all interleavings of lock/unlock
    // Verify no thread starves
}
```

**When to use:**
- Actor-based systems (supervisors, child actors)
- Lock-based concurrency
- Message-passing systems

**Guarantee:** All tested properties hold in all possible interleavings.

**Status:** Experimental (v0.4+), requires `#[cfg(feature = "stateright")]`

---

### Backend 7: Lean Interactive Proof (Future)

**Purpose:** Machine-assisted formal proofs for complex theorems.

**What it does:**
- Exports Resilient code to Lean
- Developers write Lean proofs interactively
- Lean verifier certifies the proof
- Proof is embedded in binary

**Example (future):**

```rust
#[prove_in_lean]
fn matrix_inversion_preserves_eigenvalues() {
    // Developers write Lean proof here
    // Lean verifies mathematical correctness
}
```

**When to use:**
- Mathematical correctness proofs
- Cryptographic algorithm verification
- Formal specifications of complex systems

**Status:** Future (v0.7+), requires Lean integration

---

## Verification Workflow by Use Case

### Use Case 1: Safety-Critical Embedded System

**Goal:** Guarantee zero panics, no memory safety violations, correct math.

**Workflow:**

```rust
// 1. Write contracts for every function
#[requires(x >= 0 and x <= 100)]
#[ensures(return >= x)]
fn sensor_filter(int x) -> int {
    return x;  // Simplified for example
}

// 2. Request Z3 verification for critical path
#[cfg(feature = "z3")]
#[verify]
fn critical_computation(int a, int b) -> int {
    return a * b;  // Z3 verifies no overflow
}

// 3. Test with runtime assertions (debug)
#[test]
fn test_filters() {
    assert_eq!(sensor_filter(50), 50);  // Runtime check
}

// 4. Deploy with release checks disabled (only contract signatures remain)
cargo build --release
```

**Guarantees:**
- All contracts verified at compile time
- Critical paths proven with Z3
- No panics possible (or caught at runtime in tests)
- Memory safety enforced by borrow checker

---

### Use Case 2: Distributed Consensus Protocol

**Goal:** Verify correctness under all possible message orderings.

**Workflow:**

```rust
#[model_check]  // Use TLA+ verification
fn consensus_algorithm() {
    // Verify all interleavings
    // Detect deadlocks, race conditions
    // Prove safety (all outputs agree)
}

#[stateright_test]  // Simulation testing
fn test_three_phase_commit() {
    // Explore state space
    // Verify no inconsistencies
}
```

**Guarantees:**
- All interleavings verified
- Safety properties proven
- No deadlocks or livelocks

---

### Use Case 3: General Application with Tests

**Goal:** Catch bugs during development and testing.

**Workflow:**

```rust
// 1. Write contracts for clarity
#[requires(items.len() > 0)]
fn find_max(items: &array<int>) -> int {
    // ...
}

// 2. Test with runtime assertion checking
#[test]
fn test_find_max() {
    let items = [1, 5, 3, 9, 2];
    assert_eq!(find_max(&items), 9);  // Runtime checks
}

// 3. Release without runtime checks
cargo build --release
```

**Guarantees:**
- Contracts documented and testable
- Runtime assertions catch mistakes in tests
- Zero overhead in production

---

## Feature Flags for Verification

### Enable/Disable Verification Backends

```toml
[features]
default = ["contracts"]
contracts = []           # Runtime contract checking
z3 = []                  # Z3 SMT solver verification
tla = []                 # TLA+ model checking
stateright = []          # Stateright simulation
lean = []                # Lean proof assistant
full_verification = ["z3", "tla", "stateright", "lean"]
```

**Usage:**

```bash
# Development: enable all verification
cargo test --features full_verification

# Release: only contracts, no Z3 overhead
cargo build --release --no-default-features --features contracts
```

---

## Verification Quality Metrics

### Metric 1: Coverage

What percentage of code paths are verified?

```rust
#[measure_coverage]
fn test_critical_path() {
    // Rust code
}

// Report: 95% of branches covered by Z3 / TLA+ / Stateright
```

### Metric 2: Proof Effort

How much human effort was required for formal proof?

| Backend | Effort | Proof Time |
|---------|--------|-----------|
| Contracts | Minimal (metadata) | 0 (compile) |
| Z3 | Low (assertions) | < 10s |
| TLA+ | Medium (model) | < 60s |
| Stateright | Low-Medium (annotations) | < 30s |
| Lean | High (interactive) | Hours/days |

### Metric 3: Assurance Level

What does the proof guarantee?

| Backend | Guarantee | Limitations |
|---------|-----------|-----------|
| Contracts | Preconditions met at call | Doesn't verify implementation |
| Z3 | Arithmetic correct | Timeout possible, incomplete logic |
| TLA+ | Safety property holds | Bounded state space |
| Stateright | All interleavings tested | Doesn't prove unbounded systems |
| Lean | Mathematically proven | Manual proof required |

---

## Integration Points

### Compiler Integration

```rust
// Compiler automatically:
// 1. Checks contracts at compile time
// 2. Calls Z3 if #[verify] is present
// 3. Fails build if verification fails
// 4. Embeds proof in binary (if successful)
```

### Test Framework Integration

```rust
#[test]
fn test_with_verification() {
    // Test runs with runtime assertion checking
    // Z3 proofs are precomputed (from build)
    // Stateright explores interleavings
}
```

### Performance Integration

```rust
// Release build:
// - Contracts become no-ops (inlined assertions that compile away)
// - Z3 proofs embedded as documentation
// - Runtime overhead: < 1%
```

---

## Roadmap

### v0.3: Contract Foundation

- [x] `#[requires]` and `#[ensures]` syntax
- [x] Compile-time contract checking
- [x] Runtime assertion support

### v0.4: Z3 Verification

- [x] Z3 integration (experimental)
- [ ] Automatic constraint generation
- [ ] Proof caching

### v0.5: Verification Stabilization

- [ ] Z3 contracts (Stable tier)
- [ ] TLA+ model checking (Backend-Limited)
- [ ] Stateright simulation (Experimental)

### v0.6+: Advanced Verification

- [ ] Lean integration (Future)
- [ ] Interactive proof mode
- [ ] Proof sharing / reuse
- [ ] Formal specification language

---

## Best Practices

### 1. Contract Everything (Start with Contracts)

```rust
// Good: contracts on every function
#[requires(x >= 0)]
#[ensures(return == x * x)]
fn square(int x) -> int { ... }

// Avoid: no contracts
fn square(int x) -> int { ... }
```

### 2. Z3 for Critical Paths

```rust
// Good: verify critical arithmetic
#[verify]
fn safety_critical_math(int a, int b) -> int { ... }

// Avoid: verify everything (slows build)
#[verify]
fn trivial_helper(int x) -> int { x + 1 }
```

### 3. Stateright for Concurrency

```rust
// Good: test actor systems
#[stateright_test]
fn test_supervisor_recovery() { ... }

// Avoid: relying only on contracts for concurrency
```

### 4. Separate Verification Concerns

```rust
// Good: one verification per backend
#[test] fn test_logic() { ... }              // Runtime
#[verify] fn prove_math() { ... }             // Z3
#[stateright_test] fn verify_concurrency() {} // Stateright

// Avoid: mixing all in one test
```

---

## References

- **RES-3509:** Unify the verification surface into one user-facing model
- **RES-3504:** Memory model specification
- **STABILITY_POLICY.md:** Feature tier guarantees

