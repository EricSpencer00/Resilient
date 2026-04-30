---
title: Design Philosophy
nav_order: 5
has_children: true
permalink: /philosophy
---

# Design Philosophy
{: .no_toc }

Why Resilient looks the way it does.
{: .fs-6 .fw-300 }

<details open markdown="block">
  <summary>Table of contents</summary>
  {: .text-delta }
- TOC
{:toc}
</details>

---

## Three pillars

Most language design choices in Resilient resolve to one of
three commitments. When they conflict, the order below is the
tiebreaker.

### 1. Resilience

> Failures are inevitable. Treat them as expected events, not
> exceptions.

Most languages model failure as an unwinding exception, a
sentinel return value, or a panic. All three force the
programmer to either (a) handle every failure path explicitly
at the call site, or (b) accept that any unhandled error
crashes the program.

Neither is acceptable for the targets Resilient cares about
(automotive ECUs, flight control, infusion pumps, industrial
controllers). Crashing is unsafe; explicit per-call error
handling produces error-handling code that's larger than the
business logic and itself a source of bugs.

Resilient's answer is the `live { }` block. Code inside a live
block is supervised: when it raises a recoverable error, the
runtime restores the block's local state to its
last-known-good snapshot and re-runs the block. From the
caller's perspective, the live block either eventually
succeeds or escalates after exhausting its retry budget. The
unhappy path is the runtime's problem, not the programmer's.

```rust
live {
    let frame = read_sensor();        // may transient-fail
    assert(is_valid(frame), "bad frame");
    process(frame);
}
```

This is structurally similar to Erlang's "let it crash" model
+ supervisor trees, restricted to a single block scope so the
control flow stays local and the snapshot/restore boundary is
syntactically obvious.

### 2. Verifiability

> It shouldn't just work; it must be **provably correct**.

Tests show that a program works on the inputs you wrote
tests for. They say nothing about the inputs you didn't.
For a fuel gauge, a brake controller, or an ECU, "we
didn't test that input" is not a sufficient defense.

Resilient lifts function contracts (`requires` / `ensures`)
into the language itself. The verifier tries to prove each
clause at compile time. With `--features z3`, hard clauses
get dispatched to Z3. What can't be proven becomes a runtime
check — same semantics, different cost.

```rust
fn safe_div(int a, int b)
    requires b != 0
    ensures  result * b == a
{
    return a / b;
}
```

The headline feature is **certificates** (RES-071): once Z3
discharges an obligation, the driver can dump the proof as a
self-contained SMT-LIB2 file. A safety auditor doesn't have
to trust the Resilient binary — they re-run the proof under
any compatible solver and confirm the answer themselves.

```bash
cargo run --features z3 -- --emit-certificate ./certs prog.rz
z3 -smt2 ./certs/safe_div__post__0.smt2
# unsat
```

The whole point of certificates is removing the verifier itself
from the trusted base.

### 3. Simplicity

> Minimal, unambiguous syntax. Reduce the cognitive load on the
> developer; minimize the surface area for bugs.

The language has:

- **Two statement-introducing keywords** beyond `if`/`while`/`return`:
  `let` and `fn`.
- **No macro system.** Macros are great for productivity in
  large languages and a nightmare for verification.
- **No inheritance.** No method-resolution order, no diamond
  problem, no implicit dispatch.
- **No implicit conversions.** `int + float` is a type error.
  Coerce explicitly with `to_float(x)`.
- **No null.** Optional values use a `Result`-style sum type
  (planned).
- **One way to declare a function.** No methods-vs-functions
  distinction. No anonymous-fn-vs-named-fn syntax difference.

This is in tension with ergonomics. We accept that. A
language used to fly an aircraft control surface or dose
a medication should be boring.

## What the design rejects

Some explicit non-goals, for clarity about what Resilient
**won't** become:

- **Not a systems language.** Rust already exists. Resilient
  doesn't try to compete on raw performance, manual memory
  management, or unsafe escape hatches. The target audience
  is application code on top of a small RTOS or bare metal.
- **Not a research language.** No dependent types, no effect
  systems, no algebraic-effects-as-first-class. The verifier
  is intentionally weaker than Liquid Haskell or F\* — it's
  meant to be used by a working safety-critical engineer, not
  a PL theorist.
- **Not a scripting language.** No dynamic typing, no
  reflection, no eval, no monkey-patching. The whole program
  is visible at compile time.
- **Not a GC language.** Values are stack-allocated where
  possible; the embedded build is alloc-free for `Int` /
  `Bool`, opt-in for `Float` / `String`. No tracing GC.

## Three backends, one language

Resilient ships **three execution backends** sharing one
front-end (lexer + parser + typechecker + verifier):

1. **Tree-walking interpreter** (default). Slow, easy to
   debug, easy to add new features to. The reference
   implementation — when in doubt about semantics, the
   interpreter is canonical.
2. **Bytecode VM** (`--vm`). Stack-based dispatch, ~12×
   faster than the interpreter on `fib(25)`. Same source,
   same semantics, just compiled to a simple op sequence.
3. **Cranelift JIT** (`--features jit --jit`). Real native
   code via Cranelift. ~12× faster than the VM, within ~1.4×
   of native Rust on the same workload.

Why three and not just one? Because the right backend depends
on the workload:

- A 50-line one-shot script: interpreter.
- A medium-loop server: VM (no compile-time tax).
- A long-running embedded loop or recursive numeric kernel:
  JIT (compile cost amortizes).

The user picks at the command line. The compiler doesn't
guess.

## Versioning and stability

Resilient is in active development, tracked publicly in
[GitHub Issues](https://github.com/EricSpencer00/Resilient/issues).
Each commit closes one numbered ticket
(`RES-NNN: <imperative summary>`); each ticket has concrete,
verifiable acceptance criteria. The roadmap
([ROADMAP.md](https://github.com/EricSpencer00/Resilient/blob/main/ROADMAP.md))
lists 20+ numbered goalposts (G1–G20+); the changelog at the
bottom is the source of truth for what shipped when.

We don't yet have a stable version number. Until we do, treat
the language as research-quality: small enough to fit in your
head, but evolving fast enough that a syntax tweak between
commits is normal.

## What we got wrong

Honest self-criticism (more useful than marketing):

- **Reassignment isn't yet in the JIT.** Phase G shipped
  immutable `let`. Reassignment + `while` loops are the next
  small JIT phase (RES-107). Until then, the JIT can't take
  loop-heavy workloads — use the VM.
- **The error type carries `&'static str`.** Diagnostics for
  things like "call to unknown function: NAME" can't include
  the actual name. A future ticket will widen `JitError` to
  carry owned strings.
- **No struct system yet.** `Node::StructDecl` exists in the
  AST and the interpreter handles it; the bytecode VM and JIT
  don't. Plays in goalpost G11+.
- **Closures are recognized but limited.** `FunctionLiteral`
  is parsed; the interpreter handles closures with shared
  mutation (RES-056); the JIT lowers only top-level fns
  (Phase H).
- **The "live block" snapshot semantics are document-by-example.**
  We need a precise specification of what state is rolled
  back. Today: just the local variable env. Tomorrow: needs
  to extend cleanly to embedded I/O effects (probably via an
  effect-handler-like mechanism, but no PRD ticket yet).

## Inspirations

In rough order of influence:

- **Erlang** — supervisor trees, "let it crash," the
  philosophy that fault-tolerance is a runtime concern.
- **Ada / SPARK** — contracts as first-class language
  features, formal-methods-lite for engineers.
- **Rust** — fearless refactoring via a real type system, but
  without the lifetime calculus that makes Rust hard for
  application code.
- **Lua** — small enough that the entire language fits in
  your head; embeddable.
- **Cranelift** — choosing a small, well-engineered JIT
  framework over rolling our own.

## Further reading

- [Getting Started](getting-started) — install and run your
  first program
- [Syntax Reference](syntax) — the full grammar
- [Performance](performance) — the bench numbers
- [GitHub Issues](https://github.com/EricSpencer00/Resilient/issues)
  — the live engineering ledger
