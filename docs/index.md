---
title: Home
layout: home
nav_order: 1
description: "Resilient is a statically-typed compiled language designed for extreme reliability in embedded and safety-critical systems."
permalink: /
---

# Resilient
{: .fs-9 }

A programming language designed for **extreme reliability** in embedded and safety-critical systems.
{: .fs-6 .fw-300 }

[Get started](getting-started){: .btn .btn-primary .fs-5 .mb-4 .mb-md-0 .mr-2 }
[View on GitHub](https://github.com/EricSpencer00/Resilient){: .btn .fs-5 .mb-4 .mb-md-0 }

---

## What is Resilient?

Resilient is a small, statically-typed language that takes failure as a
first-class concern. Code that runs in a `live { }` block is
supervised by the runtime; recoverable errors trigger a state
restore + retry instead of a panic. `assert` clauses and function
contracts (`requires` / `ensures`) carry information the verifier
can prove at compile time, with optional Z3 for the harder cases
and exportable SMT-LIB2 certificates so downstream consumers can
re-verify under their own solver.

The compiler ships three execution backends: a tree-walking
interpreter (default, fastest to iterate on), a stack-based
bytecode VM (~12× faster than the interpreter on `fib(25)`), and
a Cranelift JIT (~12× faster than the VM, within ~1.4× of native
Rust on the same workload). Pick the one that matches your
deploy target.

A sibling `#![no_std]` runtime crate cross-compiles to
`thumbv7em-none-eabihf` (Cortex-M4F class MCU) so the same
language can target both your laptop and a microcontroller.

## Three pillars

The whole design follows from three commitments:

- **[Resilience](philosophy#resilience)** — failures are
  expected events, not exceptions. `live { }` blocks self-heal
  on transient errors; the runtime keeps the system in a
  functional state.
- **[Verifiability](philosophy#verifiability)** — it shouldn't
  just work; it must be **provably correct**. Function contracts
  are proven at compile time when possible, checked at runtime
  otherwise.
- **[Simplicity](philosophy#simplicity)** — the syntax is small
  enough that the surface area for bugs is small too. No macro
  system, no inheritance, no implicit conversions.

[Read the full design philosophy →](philosophy)

## Performance

`fib(25)` on Apple M1 Max:

| backend                  | median  | vs interp |
|--------------------------|---------|-----------|
| Resilient (interp)       | 406.7 ms | 1×       |
| Resilient (VM)           | 33.7 ms | 12×       |
| **Resilient (JIT)**      | **2.8 ms** | **145×** |
| Rust (native -O)         | 2.0 ms  | 204×      |

[Full benchmark methodology →](performance)

## Five-minute tour

```rust
// A function with explicit parameter types, a contract, and a
// live block that retries on recoverable error.
fn safe_divide(int a, int b)
    requires b != 0
    ensures result * b == a
{
    return a / b;
}

fn main() {
    live {
        let result = safe_divide(100, 7);
        println("100 / 7 = " + result);
    }

    assert(safe_divide(50, 5) == 10, "math is broken");
}
```

The contract on `safe_divide` is proven at compile time when
the verifier (with optional `--features z3`) can show
`b != 0` ⇒ `(a / b) * b == a` for all `a, b: int`. When it
can, no runtime check is emitted. When it can't, the same
clauses become runtime asserts.

[Get started in 60 seconds →](getting-started)

## What's in the box

| Surface             | Status | Where |
|---------------------|--------|-------|
| Tree-walking interpreter | ✅ stable | `cargo run -- prog.rs` |
| Bytecode VM         | ✅ stable | `--vm prog.rs` |
| Cranelift JIT       | ✅ stable subset | `--features jit --jit prog.rs` |
| Z3 contract proofs  | ✅ opt-in | `--features z3 --audit prog.rs` |
| SMT-LIB2 certificates | ✅ opt-in | `--emit-certificate ./certs/ prog.rs` |
| Language Server (LSP) | ✅ opt-in | `--features lsp --lsp` |
| `#![no_std]` runtime | ✅ stable | sibling `resilient-runtime/` crate |

## Open source

Resilient is free and open source software released under the
**MIT License**. Contributions from humans and AI agents are
equally welcome — the ticket system in
[`.board/`](https://github.com/EricSpencer00/Resilient/tree/main/.board)
is designed to be machine-readable, and `cargo test` is the
authoritative acceptance gate.

[Contributing guide](contributing){: .btn .btn-outline .mr-2 }
[Community & Open Source](community){: .btn .btn-outline }

---

## Where next?

- **New here?** → [Getting Started](getting-started)
- **Building something?** → [Syntax Reference](syntax)
- **Writing tooling, static analysis, or safety audits?** → [Language Reference](language-reference) (formal grammar, type rules, semantics)
- **Curious about design tradeoffs?** → [Philosophy](philosophy)
- **Embedding the runtime?** → [no_std runtime](no-std)
- **Auditing memory behaviour?** → [Memory Model](memory-model)
- **Concurrency, ISRs, and real-time?** → [Concurrency and Real-Time Scheduling](concurrency)
- **DO-178C / ISO 26262 / IEC 61508 / MISRA?** → [Certification and Safety Standards](certification)
- **Setting up your editor?** → [LSP / Editor Integration](lsp)
- **Looking for a tool (fmt, lint, verify-cert, REPL, fuzz, ...)?** → [Tooling Reference](tooling)
- **Contributing?** → [Contributing guide](contributing) and [`.board/`](https://github.com/EricSpencer00/Resilient/tree/main/.board) on GitHub
- **License, community, open source?** → [Community & Open Source](community)
