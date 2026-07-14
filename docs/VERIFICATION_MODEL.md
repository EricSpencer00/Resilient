---
title: Verification Model
nav_order: 10
permalink: /verification-model
---

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

### Backend 2: Z3 SMT Solver (Opt-in Cargo feature)

**Purpose:** Formal verification of `#[requires]` / `#[ensures]` contracts using automated theorem proving.

**What it does:**
- There is no `#[verify]` attribute — Z3 is not requested per function.
  Every `#[requires]` / `#[ensures]` contract is folded by a hand-rolled
  constant solver first (`resilient/src/verifier_z3.rs`, RES-060..065);
  when folding returns `Unknown`, the compiler escalates to Z3
  automatically, but only in binaries built with `--features z3`
  (`resilient/Cargo.toml:48`: `z3 = ["dep:z3", "dep:ed25519-dalek", "dep:rand_core"]`).
- Without `--features z3` the same contract sites still typecheck against
  the hand-rolled folder alone; the Z3 escalation path is compiled out
  (`resilient/src/lib.rs:29482`, `#[cfg(not(feature = "z3"))]`).
- Contract verdicts can be exported as certificates and signed with
  `rz build --emit-certificate <dir> [--sign-cert <keyfile>]`
  (`resilient/src/lib.rs:29941` `emit_certificates`,
  `resilient/src/lib.rs:30329`), then checked out-of-line with
  `rz verify-cert <dir> [--pubkey <path>]` or `rz verify-all <dir> [--z3]`
  (`resilient/src/lib.rs:31406` `dispatch_verify_cert_subcommand`,
  `resilient/src/lib.rs:31537` `dispatch_verify_all_subcommand` — both
  `#[cfg(feature = "z3")]`).

**Example:**

```rust
// No attribute needed — Z3 (when the binary is built with it) backs
// every contract below once the hand-rolled folder can't decide it.
#[requires(x >= 0)]
#[requires(y >= 0)]
#[ensures(return >= x and return >= y)]
fn max(int x, int y) -> int {
    if x > y { return x; }
    return y;
}
```

**When to use:**
- Build with `--features z3` for critical arithmetic (safety-critical systems)
- Complex predicates with many branches that the hand-rolled folder can't decide
- When you need a signed certificate artifact for an external verification step

**Limitations:**
- Works best for decidable logic (arithmetic, bit operations)
- May timeout on very complex formulas
- Floating-point support incomplete
- Only present in `--features z3` builds; default builds skip Z3 entirely
  and rely on the hand-rolled folder plus runtime checks

**Integration:**

```bash
# Build with Z3 verification and emit signed contract certificates
cargo build --manifest-path resilient/Cargo.toml --features z3
rz build --emit-certificate ./certs --sign-cert ./signing.key program.rz
rz verify-cert ./certs --pubkey ./signing.pub
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

### Backend 5: TLA+ Model Checking (External spec, CLI-driven)

**Purpose:** Verify distributed systems and concurrent algorithms.

**What it does:**
- There is no `#[model_check]` attribute and no `tla` Cargo feature —
  `grep -rn 'feature = "tla"' resilient/Cargo.toml` finds nothing, and
  the `tla` feature does not appear in `[features]`
  (`resilient/Cargo.toml:39-99`).
- TLA+ verification is external-spec, not in-language: you write a
  standalone `.tla` specification and check it with
  `rz tla check <file.tla>`, which shells out to TLC
  (`java -jar tla2tools.jar`) and translates its output into Resilient's
  diagnostic format
  (`resilient/src/tla_bridge.rs:1-20`, `resilient/src/lib.rs:33092`
  `tla_bridge::is_tla_check_help_request`,
  `resilient/src/lib.rs:33106` `tla_bridge::dispatch_tla_subcommand`).
- The bridge is compiled into every non-wasm build unconditionally —
  it is `#[cfg(not(target_arch = "wasm32"))]`, not feature-gated
  (`resilient/src/lib.rs:154`). Without a `tla2tools.jar` on `PATH`,
  `--tlc-jar`, or `RESILIENT_TLC_JAR`, the command prints a clear
  "not available" message and exits non-zero — it never panics.

**Example:**

```bash
rz tla check MySpec.tla
# or, with an explicit tla2tools.jar:
rz tla check --tlc-jar ./tla2tools.jar MySpec.tla
```

**When to use:**
- Consensus protocols
- Multi-actor systems
- Distributed algorithms

**Guarantee:** If `rz tla check` reports no violations, TLC found no
interleaving of the modeled spec that violates the invariant. The
Resilient source is not translated to TLA+ automatically — you author
the `.tla` model yourself.

**Status:** Shipped (CLI subcommand, `resilient/src/tla_bridge.rs`, 545
lines including tests); requires Java + `tla2tools.jar` at runtime, not
a Cargo feature.

---

### Backend 6: Stateright Simulation Testing (Opt-in Cargo feature, CLI-driven)

**Purpose:** Test concurrent Resilient `actor` definitions with exhaustive state exploration.

**What it does:**
- There is no `#[stateright_test]` attribute. The `stateright` Cargo
  feature is real (`resilient/Cargo.toml:99`: `stateright = ["dep:stateright"]`,
  `dep:stateright` pinned to `0.31.0`), but it gates a CLI subcommand,
  not a function-level annotation.
- `rz stateright check <file.rz>` parses the file's `actor { ... }`
  definitions and exhaustively explores interleavings of their
  `receive` handlers looking for `always:` invariant violations
  (`resilient/src/stateright_bridge.rs:98`
  `dispatch_stateright_subcommand`, `resilient/src/stateright_bridge.rs:227`
  `check_source`).
- The bridge is only compiled for non-wasm targets built with
  `--features stateright`
  (`resilient/src/lib.rs:151`:
  `#[cfg(all(not(target_arch = "wasm32"), feature = "stateright"))]`).

**Example:**

```rust
// bounded.rz
actor Q {
    state: int = 0;
    always: state <= 2;
    receive push() requires state < 2 { self.state = self.state + 1; }
    receive pop() requires state > 0 { self.state = self.state - 1; }
}
```

```bash
cargo build --manifest-path resilient/Cargo.toml --features stateright
rz stateright check bounded.rz
```

**When to use:**
- Actor-based systems (supervisors, child actors)
- Lock-based concurrency
- Message-passing systems

**Guarantee:** All `always:` invariants hold across every explored
interleaving of the actor's `receive` handlers.

**Status:** Shipped behind `--features stateright`
(`resilient/src/stateright_bridge.rs`, 666 lines including tests); no
in-language attribute exists or is planned.

---

### Backend 7: Lean (External tool via MCP bridge only)

**Purpose:** Machine-assisted formal proofs for complex theorems.

**What it does:**
- There is no `#[prove_in_lean]` attribute, no `lean` Cargo feature, and
  no Resilient-to-Lean code export. `grep -rn '"lean"' resilient/src`
  finds exactly one hit: an MCP external-tool adapter that shells out to
  a standalone `lean` binary against a hand-written `.lean` file
  (`resilient/src/mcp_tool_registry.rs:274` `lean4_adapter`,
  `binary_name: "lean".to_string()`, discoverable via the
  `RESILIENT_LEAN_BIN` environment variable).
- This adapter is part of the MCP tool-bridge registry
  (`resilient/src/mcp_tool_registry.rs`), which lets an MCP client ask
  Resilient's tooling to run Lean 4 proof checking on an existing
  `.lean` file — it does not generate Lean from Resilient source, embed
  a proof in the compiled binary, or gate any part of the compiler.

**Example:**

```rust
// MCP tool-registry request shape (not Resilient source syntax):
// { "tool": "lean4_check", "file": "proof.lean", "theorem": "eigen_preserved" }
```

**When to use:**
- Mathematical correctness proofs, authored directly in Lean, that you
  want an MCP-connected agent/editor to check via the bridge

**Status:** MCP external-tool adapter only
(`resilient/src/mcp_tool_registry.rs`); no in-language attribute, no
Cargo feature, no code export, and no proof embedding exist or are
implemented.

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

// 2. Build with --features z3 so the same contract below is
//    escalated to Z3 automatically when the hand-rolled folder
//    can't decide it — no attribute needed.
#[requires(a > 0)]
#[ensures(return >= a)]
fn critical_computation(int a, int b) -> int {
    return a * b;  // Z3 verifies no overflow, given --features z3
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

```bash
# 1. Author a standalone TLA+ spec of the protocol, then check it
rz tla check consensus.tla

# 2. Model the protocol's actors directly in Resilient and let
#    Stateright explore their interleavings (requires --features stateright)
rz stateright check three_phase_commit.rz
```

**Guarantees:**
- `rz tla check`: no interleaving of the *modeled* `.tla` spec violates
  the invariant (bounded by TLC's state space)
- `rz stateright check`: no interleaving of the actors' `receive`
  handlers violates an `always:` invariant
- Neither tool derives its model from your Resilient source
  automatically — TLA+ specs are hand-authored; Stateright checks the
  actor definitions you already wrote

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

### Actual `[features]` in `resilient/Cargo.toml`

There is no `contracts`, `tla`, `lean`, or `full_verification` feature.
Contract checking (`#[requires]` / `#[ensures]`) is core language
semantics, not feature-gated. TLA+ (`rz tla check`) is compiled into
every non-wasm build unconditionally. The only verification-relevant
opt-in Cargo features are `z3` and `stateright`
(`resilient/Cargo.toml:39-99`):

```toml
[features]
default = []
z3 = ["dep:z3", "dep:ed25519-dalek", "dep:rand_core"]
stateright = ["dep:stateright"]
```

**Usage:**

```bash
# Development: enable Z3 escalation + Stateright actor model checking
cargo test --manifest-path resilient/Cargo.toml --features z3,stateright

# Default: contracts + runtime assertions + rz tla check, no Z3 overhead
cargo build --manifest-path resilient/Cargo.toml --release
```

---

## Verification Quality Metrics

### Metric 1: Coverage

What percentage of code paths are verified?

There is no `#[measure_coverage]` attribute or automated coverage report
across backends — `grep -rn measure_coverage resilient/src` finds
nothing. Today this is a manual accounting exercise: count functions
with `#[requires]`/`#[ensures]`, count how many are in a `--features z3`
build (so folding can escalate to Z3), and separately track which
`.tla` specs and `actor` definitions have `rz tla check` /
`rz stateright check` passing in CI.

### Metric 2: Proof Effort

How much human effort was required for formal proof?

| Backend | Effort | Proof Time |
|---------|--------|-----------|
| Contracts | Minimal (`#[requires]`/`#[ensures]` metadata) | 0 (compile) |
| Z3 | Low (write contracts; `--features z3` does the rest) | < 10s |
| TLA+ | Medium (author a standalone `.tla` model by hand) | < 60s |
| Stateright | Low-Medium (write `actor { ... always: ... }` defs) | < 30s |
| Lean | High (write the `.lean` proof yourself; MCP bridge only checks it) | Hours/days |

### Metric 3: Assurance Level

What does the proof guarantee?

| Backend | Guarantee | Limitations |
|---------|-----------|-----------|
| Contracts | Preconditions met at call | Doesn't verify implementation |
| Z3 | Arithmetic correct (`--features z3` builds only) | Timeout possible, incomplete logic |
| TLA+ | Safety property holds for the *modeled* `.tla` spec | Bounded state space; model is hand-authored, not derived from source |
| Stateright | All interleavings of the actor's `receive` handlers tested (`--features stateright`) | Doesn't prove unbounded systems |
| Lean | Mathematically proven, if the `.lean` proof was written and passes | Manual proof required; not connected to Resilient source at all — MCP bridge only shells out to `lean` on an existing `.lean` file |

---

## Integration Points

### Compiler Integration

```rust
// Compiler automatically:
// 1. Checks #[requires] / #[ensures] contracts at compile time
// 2. Folds them with the hand-rolled solver; escalates to Z3 only
//    if the binary was built with --features z3 (no attribute needed)
// 3. Fails build if a contract is statically violated
// 4. Optionally emits signed contract certificates via
//    `rz build --emit-certificate <dir> [--sign-cert <keyfile>]`
```

### Test Framework Integration

```rust
#[test]
fn test_with_verification() {
    // Test runs with runtime assertion checking of contracts.
    // Z3-backed contract verdicts (--features z3) were computed at
    // compile time, not re-run per test.
    // Stateright interleaving exploration is a separate CLI step
    // (`rz stateright check`), not part of #[test].
}
```

### Performance Integration

```rust
// Release build:
// - Z3 (--features z3) verdicts are computed at compile time only —
//   there is no runtime Z3 call, and no "proof embedded as
//   documentation" artifact is generated
```

---

## Roadmap

### v0.3: Contract Foundation

- [x] `#[requires]` and `#[ensures]` syntax
- [x] Compile-time contract checking
- [x] Runtime assertion support

### v0.4: Z3 Verification

- [x] Z3 escalation for `#[requires]`/`#[ensures]` contracts (`--features z3`)
- [x] `rz verify-cert` / `rz verify-all` certificate verification CLI
- [ ] Automatic constraint generation beyond current fold + Z3 escalation
- [ ] Proof caching

### v0.5: Verification Stabilization

- [x] `rz tla check` — external `.tla` spec checking via TLC (shipped, not feature-gated)
- [x] `rz stateright check` — actor interleaving exploration (`--features stateright`)
- [ ] Z3 contracts promoted to Stable tier (see STABILITY.md)

### v0.6+: Advanced Verification

- [ ] Deeper MCP-bridged Lean workflow (today: `lean4_check` MCP tool
      shells out to an external `lean` binary on a hand-written
      `.lean` file — no Resilient-to-Lean export exists)
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

### 2. Build with `--features z3` for Critical Paths

```rust
// Good: strong contracts on critical arithmetic, built with --features z3
// so the compiler escalates to Z3 whenever the hand-rolled folder can't decide
#[requires(a > 0 && b > 0)]
#[ensures(return >= a && return >= b)]
fn safety_critical_math(int a, int b) -> int { ... }

// Avoid: no contracts at all on critical paths — there's nothing for
// either the folder or Z3 to check
fn trivial_helper(int x) -> int { x + 1 }
```

### 3. Model Actor Systems for Stateright

```rust
// Good: model the system as actor { ... always: ... } and run
// `rz stateright check` with --features stateright
actor Supervisor {
    state: int = 0;
    always: state >= 0;
    receive recover() requires state >= 0 { self.state = 0; }
}

// Avoid: relying only on contracts for concurrency — contracts check
// single-call preconditions, not cross-actor interleavings
```

### 4. Separate Verification Concerns

```rust
// Good: one verification surface per backend
#[test] fn test_logic() { ... }  // Runtime assertion checking
// `rz build --features z3` backs the contracts above at compile time
// `rz stateright check` explores actor interleavings separately

// Avoid: expecting a single #[test] to cover contracts, Z3, and
// Stateright simultaneously — each is a distinct build/CLI step
```

---

## References

- **RES-3509:** Unify the verification surface into one user-facing model
- **RES-3504:** Memory model specification
- **STABILITY.md:** Feature tier guarantees
