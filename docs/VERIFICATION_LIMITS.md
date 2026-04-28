# Formal Verification: Scope and Limitations

Resilient's `requires` and `ensures` system proves correctness **within a mathematical model**. This document clarifies what is proven, what is not, and how to use formal verification safely in real-world systems.

## Core Truth

Formal verification can prove: "If this code is called correctly, in a single-threaded context, on a perfectly functioning machine, with a correct specification — then the property holds."

What it **cannot** guarantee: that any of those preconditions actually hold in production.

---

## Five Critical Limitations

### 1. The Specification May Be Wrong

Your `requires` and `ensures` clauses reflect your *model* of what the code should do — not necessarily what it should actually do.

**Example: A banking system**

```rust
fn transfer(from: i32, to: i32, amount: i32, accounts: [i32]) -> [i32]
ensures total_balance(result) == total_balance(accounts);
{
    // ... implementation ...
}
```

This proof guarantees that money is conserved **in the model**. But the model is incomplete:

- **Fees**: Banks charge transaction fees. Your model has no fee account.
- **Interest**: New money can appear from interest and dividends.
- **Rounding**: Real currencies have decimal precision (`$0.01` increments). Integer arithmetic loses cents.
- **Regulatory requirements**: Regulatory holds, suspicious-activity blocks, and risk limits are not modeled.

The code is a correct implementation of an **imperfect specification**. The proof is mathematically sound but practically misleading.

**Lesson**: Verify critical invariants, not entire application logic. Formal proofs shine for small, well-specified kernels (cryptographic primitives, memory allocators, schedulers) — not for business rules.

---

### 2. Concurrency Is Not Modeled

`requires` and `ensures` prove sequential correctness. Multi-threaded execution bypasses all guarantees.

**Example:**

```rust
fn transfer(from: i32, to: i32, amount: i32, accounts: [i32]) -> [i32] {
    // Prove: money is conserved
}
```

The proof covers a single call. It assumes:

- The function runs alone.
- Reads and writes appear to happen instantaneously.
- No other thread accesses `accounts` during execution.

In reality:

1. Thread A reads `accounts[from].balance` → sees `$100`
2. Thread B reads `accounts[from].balance` → sees `$100`
3. Thread A transfers `$100` (balance now `$0`)
4. Thread B transfers `$100` (balance becomes `-$100`)
5. **Double-spend**: The same `$100` was sent twice.

The formal proof covered Thread A's execution perfectly. The concurrent scenario was never verified.

**Lesson**: Formal verification + concurrency = unsafe. Layer concurrency controls *outside* verified code (locks, atomic operations, message queues) and keep the verified region single-threaded.

---

### 3. The Hardware Doesn't Care About Proofs

Formal proofs assume perfect hardware. Real machines fail silently.

**Integer Overflow:**

```rust
fn total_balance(accounts: [i32]) -> i32 {
    let mut sum: i32 = 0;
    // loop invariant: sum == sum of balances seen so far
    for account in accounts {
        sum = sum + account.balance;  // Proof: sum stays correct
    }
    sum  // Proof: sum is the total
}
```

If `accounts` contains very large balances, `sum` silently wraps (e.g., from `2^31 - 1` to `-2^31`). The proof never considered this — it assumed arithmetic is exact. In the real world, an overflow corrupts the invariant.

**Memory Corruption:**

- A stray write from another process overwrites your data.
- A cosmic ray flips a bit in RAM.
- Cache coherency issues on NUMA architectures.

**Storage Failures:**

- A crash mid-write leaves the data partially updated.
- A corrupted sector is silently read as different data.

**Lesson**: Formal proofs assume the machine is honest. Use hardware-level defenses (parity checks, checksums, watchdog timers) to protect verified code.

---

### 4. The Boundary Between Verified and Unverified Code Is Fragile

`transfer` might be formally proven, but it exists within an unproven system.

**The Unverified Layers:**

```
HTTP Endpoint (unverified)
    ↓
REST Deserializer (unverified)
    ↓
Database ORM (unverified)
    ↓
transfer() ← [VERIFIED]
    ↓
Database write-back (unverified)
    ↓
Message queue replay (unverified)
```

Bugs in any unverified layer completely bypass the formal guarantee:

- **Wrong index**: `accounts[from]` accesses the wrong account.
- **Deserialization error**: Malformed JSON is parsed as a different `amount`.
- **Duplicated message**: The transfer request is retried; the same transaction runs twice.
- **Wrong version deployed**: `buggy_transfer` is deployed instead of the verified one.

**Lesson**: Verify the critical kernel, but don't pretend the surrounding system is safe. Test integration points between verified and unverified code with **at least as much rigor as the proof itself** — the integration is usually where bugs hide.

---

### 5. Verification Only Covers What's Written

The proof cannot prevent what you deploy tomorrow.

A single file might contain both:

```rust
fn transfer(...) -> [...] ensures total_balance(result) == total_balance(accounts) { ... }

fn buggy_transfer(...) -> [...] { 
    // Missing conservation check — silently loses money
    ...
}
```

Nothing in the language prevents developers from:

- Adding a new, unverified function.
- Using the wrong function in production (configuration error, wrong build artifact).
- Removing the verification annotation to "ship faster."

**Lesson**: Formal verification is part of a **culture**, not a substitute for code review, testing, and deployment discipline.

---

## Real-World Verification Success Stories

Formal verification works best on:

- **seL4 microkernel**: ~10,000 lines of kernel code, fully proven for binary equivalence under a well-defined threat model. **Success** because the scope was tightly bounded and the model was realistic.
- **AWS s2n-tls**: TLS library with mechanized proofs of specific cryptographic properties. **Success** because cryptography has a precise, well-understood model.
- **Ethereum smart contracts**: Hundreds of formal proofs of contract invariants. **Mixed results**: proofs often don't capture economic incentives or multi-contract interactions.

---

## Using Formal Verification Safely

### ✅ Do

- Verify **small, critical kernels**: cryptographic primitives, memory allocators, schedulers, consensus protocols.
- Write **tight, realistic specifications**: the spec should reflect what actually matters.
- **Pair verification with testing**: proofs prove the model; tests probe reality.
- **Keep the verified region single-threaded**: handle concurrency outside the proof boundary.
- **Defend the hardware**: checksums, parity, watchdog timers, redundancy.
- **Document assumptions**: list everything the proof assumes and where it might break.

### ❌ Don't

- Verify entire applications. (Proofs are expensive; focus on the kernel.)
- Trust the spec without reality-checking it. (Walk through production failure modes.)
- Claim "mathematically guaranteed safety" in marketing. (You've proven the model, not the world.)
- Ignore concurrency. (Formal proofs are sequential by default.)
- Skip testing integration points. (This is where bugs actually live.)
- Assume the proof prevents human error. (Configuration, deployment, and operational mistakes still happen.)

---

## Resilient's Approach

Resilient offers `requires` and `ensures` annotations to enable **targeted formal verification of critical functions**. The language itself does not make strong safety claims beyond what's proven:

- ✓ Memory safety: enforced by the type system and borrow checker.
- ✓ Type safety: enforced at compile time.
- ✓ Formal properties of individual functions: proven via `requires`/`ensures` (with the caveats above).
- ✗ Whole-program safety: not guaranteed.
- ✗ Concurrency safety: not covered by proofs (use language-level concurrency primitives).
- ✗ Hardware faults: not modeled (use external defenses).

Use Resilient's formal capabilities wisely: as one tool among many (testing, code review, monitoring, redundancy) in building reliable systems.

---

## Further Reading

- **"The Verification of a Realistic Compiler"** (Leroy et al., 2009): CompCert compiler. Shows the cost and benefit of full-system verification.
- **"seL4: Formal Verification of an Operating-System Kernel"** (Klein et al., 2009): ~10k lines proven; took 20 engineer-years.
- **"Why 3 of Everything"** (Lamport, 2005): On formal methods and their practical limits.
- **"Formal Verification of Security-Critical Software"** (Woodcock et al., 2009): Survey of challenges and successes.
