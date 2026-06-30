---
title: Failure Model
parent: Design Philosophy
nav_order: 6
permalink: /failure-model
---

# Resilient Failure Model

## Overview

Resilient's failure model is designed for safety-critical embedded systems. It unifies compile-time guarantees, runtime diagnostics, recoverable faults, and explicit recovery semantics into one coherent framework.

The model answers four core questions:
1. **What can fail?** (contract violations, invalid operations, resource exhaustion)
2. **How does it fail?** (panics, errors, recoverable exceptions, proof of safety)
3. **What can recover?** (explicit `live {}` blocks, error handling, fault tolerance)
4. **What is statically guaranteed?** (memory safety, type safety, provable contracts)

---

## Failure Categories

### Category 1: Compile-Time Guarantees (No Runtime Failure)

**Definition:** Errors provably impossible at compile time.

**Guarantee:** Code that passes typechecking is guaranteed not to produce these errors at runtime.

**Examples:**
- Memory safety violations (use-after-free, double-free)
- Type mismatches in expressions
- Dangling pointer dereferences
- Array out-of-bounds access (statically known)
- Missing function parameters

**Mechanism:**
- Rust-like borrow checker
- Type system constraints
- Bounds checking at compile time
- Ownership and lifetime rules

**Error reporting:** Compiler diagnostic, non-recoverable.

---

### Category 2: Runtime Errors (Recoverable via `Result`/`try`)

**Definition:** Errors that occur at runtime but can be caught and handled.

**Guarantee:** Error is either handled via `Result` / `try` or causes program termination.

**Examples:**
- Division by zero
- Array access (dynamically unknown bounds)
- Integer overflow (checked arithmetic)
- File I/O failures
- Network timeouts
- Resource allocation failures

**Mechanism:**
- `Result<T, E>` return types
- `try` expressions for error propagation
- Match expressions for error handling

**Example:**
```rust
fn safe_divide(a: int, b: int) -> Result<int, string> {
    if b == 0 {
        return Err("division by zero");
    }
    return Ok(a / b);
}

fn main() {
    match safe_divide(10, 2) {
        Ok(result) => { print(result); }
        Err(msg) => { eprintln(msg); }
    }
}
```

**Error reporting:** `Result` or panic (if unhandled).

---

### Category 3: Recoverable Faults (via `live {}` blocks)

**Definition:** Failures from which execution can recover and continue from a known safe state.

**Guarantee:** Faults trigger explicit recovery code; without recovery, execution terminates.

**Examples:**
- Invalid input format
- Expected data not received
- Sensor reading out of range
- Configuration mismatch
- Transient hardware faults

**Mechanism:**
- `live { attempt } recover { recovery_code }` blocks
- Automatic state snapshots and rollback
- Fault injection testing with Stateright

**Example:**
```rust
live {
    sensor_reading = read_sensor();  // May return bad value
    if sensor_reading < MIN || sensor_reading > MAX {
        fault "invalid sensor";
    }
} recover {
    sensor_reading = default_value;
    log_fault("sensor out of range");
}

process(sensor_reading);  // Always valid after recover
```

**Guarantees:**
1. If `attempt` succeeds, execution continues normally
2. If `attempt` signals fault, `recover` executes
3. After recovery, state is guaranteed to be valid per contract
4. No partial state leaks between attempt and recovery

**Error reporting:** Fault signal + recovery execution + continue.

---

### Category 4: Contract Violations (Checked at Entry/Exit)

**Definition:** Precondition and postcondition violations on functions.

**Guarantee (with Z3 verifier):** 
- If verified: no contract violation is possible
- If not verified: violations cause runtime error with diagnostic

**Example:**
```rust
#[requires(x >= 0)]
#[requires(y >= 0)]
#[ensures(return >= x and return >= y)]
fn max(int x, int y) -> int {
    if x > y { return x; }
    return y;
}
```

**Precondition violation:** Caller passes invalid arguments → runtime error

**Postcondition violation:** Function returns invalid result → runtime error

**Mechanism:**
- Contract assertions at function entry/exit
- Optional SMT verification (Z3) for static proof
- Runtime checking when not verified

**Error reporting:** Contract assertion failure + diagnostic.

---

### Category 5: Panic (Unrecoverable System Failure)

**Definition:** A state from which safe recovery is impossible.

**Guarantee:** Program terminates immediately with diagnostic.

**Examples:**
- Invariant violation
- Unreachable code path
- Stack overflow
- Out-of-memory (allocator failure)

**Mechanism:**
- `panic!()` macro
- Invariant violation detection
- Stack/memory exhaustion detection
- Uncaught exceptions

**Rule:** Panics should not occur in production-grade code. If a panic is possible, it's a bug that should be fixed via proper error handling.

**Error reporting:** Panic message + stack trace + termination.

---

## Effect Annotations and Failure

### Pure Functions (No Effects)

```rust
fn add(int x, int y) -> int {
    return x + y;  // No failures possible
}
```

**Guarantee:** Function cannot fail (no I/O, no allocation, no division).

---

### I/O Functions

```rust
fn read_file(path: string) -> Result<string, string> ! IO {
    // May fail: file not found, permission denied, read error
}
```

**Guarantee:** Failures are only via `Result` or panic.

---

### Mutation Functions

```rust
fn update_counter(counter: &mut int) -> unit ! Mutation {
    *counter += 1;  // Cannot fail (mutation is atomic)
}
```

**Guarantee:** Mutation itself cannot fail; side effects are atomic from caller's perspective.

---

## Error Handling Patterns

### Pattern 1: Propagate Errors with `try`

```rust
fn read_and_process(path: string) -> Result<string, string> {
    let data = try read_file(path);     // Errors propagate
    let result = try parse_data(data);  // Errors propagate
    return Ok(result);
}
```

---

### Pattern 2: Recover with `live {}`

```rust
live {
    let value = fetch_from_network();
    if value < 0 {
        fault "invalid network value";
    }
} recover {
    value = cached_value;
}

process_value(value);  // Guaranteed to be valid
```

---

### Pattern 3: Convert Panics to Errors

```rust
fn safe_divide(a: int, b: int) -> Result<int, string> {
    if b == 0 {
        return Err("division by zero");
    }
    return Ok(a / b);
}
```

---

### Pattern 4: Contract-Based Safety

```rust
#[requires(items.len() > 0)]
#[ensures(return >= 0)]
fn find_minimum(items: &array<int>) -> int {
    let min = items[0];
    for i in 1..items.len() {
        if items[i] < min {
            min = items[i];
        }
    }
    return min;
}

// Caller must ensure non-empty array
fn main() {
    let data = [1, 2, 3];
    let min = find_minimum(&data);  // Precondition satisfied
}
```

---

## Verification and Proof

### Static Verification (Z3-based)

```rust
#[verify]  // Request formal verification
fn safe_add(x: int, y: int) -> int {
    return x + y;
}
```

**After verification:**
- `safe_add` is proven to never overflow (for bounded inputs)
- Z3 generates a proof
- No runtime checking needed for this function

---

### Runtime Checking (Fallback)

```rust
#[requires(x >= 0 and y >= 0)]
fn checked_add(int x, int y) -> int {
    // Runtime check: x >= 0 and y >= 0
    return x + y;  // May overflow but precondition verified
}
```

---

## Safety Guarantees

### Memory Safety

| Guarantee | Mechanism |
|-----------|-----------|
| No use-after-free | Borrow checker + lifetime rules |
| No double-free | Ownership model |
| No dangling pointers | Reference lifetime checking |
| No data races | Exclusive access rules + effect tracking |
| No buffer overflows | Bounds checking + static analysis |

### Type Safety

| Guarantee | Mechanism |
|-----------|-----------|
| No type mismatches | Compile-time type checking |
| No invalid casts | Type system constraints |
| No null pointer dereferences | Non-null by default (Option for nullable) |
| No uninitialized variables | Must-init checking |

### Concurrency Safety

| Guarantee | Mechanism |
|-----------|-----------|
| No data races | Exclusive vs. shared access rules |
| No deadlocks (future) | Effect types + Stateright verification |
| No use-after-free in concurrent code | Lifetime + borrow rules |

---

## Failure Timeline

```
┌─────────────────┐
│ Function called │
└────────┬────────┘
         │
         v
┌─────────────────────────────┐
│ Check preconditions         │
│ (if @requires present)      │
└────────┬──────────┬─────────┘
         │          │
         │ FAIL     │ OK
         v          │
      Error         │
    (Contract)      │
                    v
            ┌──────────────────┐
            │ Execute function │
            │ body             │
            └────┬────┬────┬───┘
                 │    │    │
         ┌───────┘    │    └──────┐
         v            v           v
      Success    Error/Fault   Panic
      (Normal)   (Recoverable)  (Fatal)
         │            │          │
         v            v          v
   ┌──────────┐  ┌────────────┐ ┌──────┐
   │Continue  │  │Live/Recover│ │Abort │
   └────┬─────┘  │or try/match│ └──────┘
        │        └────────────┘
        v
┌─────────────────┐
│ Check           │
│ postconditions  │
└────┬────────┬───┘
     │        │
     │ OK     │ FAIL
     v        v
   Return   Error
           (Contract)
```

---

## When to Use Each Failure Mode

| Scenario | Recommended | Why |
|----------|-------------|-----|
| File not found | `Result<T, Error>` | Expected, recoverable |
| Invalid JSON | `Result<T, Error>` + `try` | Expected, needs specific handling |
| Sensor out of range | `live {} recover {}` | Transient, needs fallback |
| Precondition violated | Contract assertion | Caller bug, should not happen |
| Out of memory | Panic | System failure, cannot recover |
| Network timeout | `Result<T, Error>` | Expected in distributed systems |
| Invariant broken | `panic!()` | Internal logic error |
| User input mismatch | `live {} recover {}` | Transient, auto-recovery |

---

## Best Practices

### 1. Be Explicit About Failure

```rust
// Good
fn parse_int(s: string) -> Result<int, string> {
    // ... parsing logic ...
}

// Avoid: silent fallback
fn parse_int_unsafe(s: string) -> int {
    // may panic on bad input
}
```

### 2. Prefer Errors Over Panics in Libraries

```rust
// Library function: use Result
pub fn query_database(sql: string) -> Result<Data, string> {
    // Return errors, don't panic
}

// Application code: can panic if unrecoverable
fn main() {
    let data = query_database("SELECT ...")
        .expect("database must be accessible");
}
```

### 3. Use `live {}` for Transient Faults

```rust
// Good: recover from transient sensor failure
live {
    reading = sensor.read();
} recover {
    reading = last_known_good_value;
}

// Avoid: silently ignoring failures
let reading = sensor.read_or_zero();  // Hides real problems
```

### 4. Document Contract Assumptions

```rust
#[requires(list.len() > 0, "list must not be empty")]
fn head<T>(list: &array<T>) -> T {
    return list[0];
}
```

### 5. Test Error Paths

```rust
#[test]
fn test_division_by_zero() {
    let result = safe_divide(10, 0);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("division"));
}
```

---

## Roadmap

### v0.3: Enhance Error Messages

- [ ] Better contract violation diagnostics
- [ ] Suggestions for fixing common errors
- [ ] Error backtrace with function names

### v0.4: Recovery Semantics

- [ ] Formalize `live {} recover {}` semantics with Stateright
- [ ] Add fault injection testing
- [ ] Document recovery patterns

### v0.5: SMT Verification

- [ ] Z3 verification for all contract forms
- [ ] Automatic precondition inference
- [ ] Proof generation and storage

### v0.6+: Future Directions

- [ ] Exception types (checked exceptions)
- [ ] Async error handling (Future<T, E>)
- [ ] Distributed system failure modes

---

## References

- **RES-3505:** Consolidate the failure and recovery semantics
- **RES-3504:** Specify and enforce the memory model
- **MEMORY_MODEL.md:** Memory safety model
- **TYPE_SYSTEM_ROADMAP.md:** Type system design
