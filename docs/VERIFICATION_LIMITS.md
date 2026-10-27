# Formal Verification: Scope and Limitations

Resilient's verifier can check supported `requires` and `ensures` obligations
**within a mathematical model**. Merely declaring them does not prove them.
Agent-written code can be excellent; human authorship supplies no guarantee.
Review requirements, assumptions, implementation conformance and the actual
properties checked. The [assurance provisions](ASSURANCE_PROVISIONS.md) give a
concrete example and the evidence to retain.

## Core Truth

Formal verification establishes stated properties under explicit assumptions
about a model or implementation. Its scope depends on the verification method
and evidence. A conforming implementation can faithfully implement a wrong or
incomplete specification; a correct specification can also have a nonconforming
implementation.

Review separately whether assumptions hold in production. A TLC pass applies to
the configured model, enabled properties and exploration performed; it does not
alone establish all intended requirements, unrestricted system sizes, or code
correctness. TLA+ supports refinement reasoning, and additional implementation
verification can connect code to a spec. Identify that evidence rather than
assuming it from model checking.

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

**Lesson**: Review whether the properties capture the intended business rules
and failure modes. Proof of a conservation invariant alone does not establish
all transfer requirements. Choose a verification scope whose assumptions and
coverage can be reviewed.

---

### 2. A Sequential Proof Does Not Establish Concurrent Behavior

A sequential function-contract argument does not automatically cover interference
from other threads. Concurrency can be modeled and verified, but requires
appropriate state, atomicity, interleaving and fairness assumptions.

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

**Lesson**: Review synchronization and concurrent behavior explicitly. TLA+
models can check concurrent protocols; connecting them to the implementation
requires conformance evidence. Sequential function results alone do not supply it.

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
- **Model interference explicitly** when claiming concurrent properties; otherwise keep the claim scoped to the sequential context.
- **Defend the hardware**: checksums, parity, watchdog timers, redundancy.
- **Document assumptions**: list everything the proof assumes and where it might break.

### ❌ Don't

- Claim application-wide verification from a proof of one kernel or model.
- Trust the spec without reality-checking it. (Walk through production failure modes.)
- Claim "mathematically guaranteed safety" in marketing. (You've proven the model, not the world.)
- Infer concurrent correctness from an argument that only models sequential execution.
- Skip testing integration points. (This is where bugs actually live.)
- Assume the proof prevents human error. (Configuration, deployment, and operational mistakes still happen.)

---

## Resilient's Approach

Resilient offers `requires` and `ensures` annotations to enable **targeted formal verification of critical functions**. The language itself does not make strong safety claims beyond what's proven:

- ✓ Memory safety: enforced by the type system and borrow checker.
- ✓ Type safety: enforced at compile time.
- ✓ Supported function obligations: use actually discharged verifier results for the identified source and configuration, with the caveats above.
- ✗ Whole-program safety: not guaranteed.
- ✗ Whole-system concurrency safety: not established by sequential contracts alone; review the concurrent model and implementation evidence.
- ✗ Hardware faults: not modeled (use external defenses).

Use Resilient's formal capabilities wisely: as one tool among many (testing, code review, monitoring, redundancy) in building reliable systems.

---

## Further Reading

- **"The Verification of a Realistic Compiler"** (Leroy et al., 2009): CompCert compiler. Shows the cost and benefit of full-system verification.
- **"seL4: Formal Verification of an Operating-System Kernel"** (Klein et al., 2009): ~10k lines proven; took 20 engineer-years.
- **"Why 3 of Everything"** (Lamport, 2005): On formal methods and their practical limits.
- **"Formal Verification of Security-Critical Software"** (Woodcock et al., 2009): Survey of challenges and successes.
