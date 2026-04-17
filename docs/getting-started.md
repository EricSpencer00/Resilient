---
title: Getting Started
nav_order: 2
permalink: /getting-started
---

# Getting Started
{: .no_toc }

A 60-second tour of installing Resilient, running your first
program, and picking the right backend for your workload.
{: .fs-6 .fw-300 }

<details open markdown="block">
  <summary>Table of contents</summary>
  {: .text-delta }
- TOC
{:toc}
</details>

---

## Install

Resilient is a Rust project — clone and build with cargo.

```bash
git clone https://github.com/EricSpencer00/Resilient.git
cd Resilient/resilient
cargo build --release
# Binary lands at: resilient/target/release/resilient
```

There are four feature configs depending on what you need:

| Build              | Adds                                | Use when                              |
|--------------------|-------------------------------------|---------------------------------------|
| default            | interpreter + bytecode VM           | Day-to-day development                |
| `--features z3`    | Z3-backed contract proofs           | You have hard `requires`/`ensures` clauses |
| `--features lsp`   | Language Server Protocol            | Editor integration (red squiggles)    |
| `--features jit`   | Cranelift JIT                       | Hot-loop / long-running workloads     |

You can stack them: `cargo build --release --features "z3 lsp jit"`.

## Hello, world

```rust
// hello.res — Resilient source extension is .rs in this repo
// (we use Rust's extension to get free editor highlighting; the
// language is unrelated)
println("Hello, Resilient!");
```

Run it:

```bash
resilient hello.res
# → Hello, Resilient!
```

## A real program

Two functions, a contract, a `live` block, and an assert. Save
as `safe_div.rs`:

```rust
fn safe_divide(int a, int b)
    requires b != 0
    ensures  result * b == a
{
    return a / b;
}

fn main() {
    live {
        let r = safe_divide(100, 7);
        println("100 / 7 = " + r);
    }

    assert(safe_divide(50, 5) == 10, "math is broken");
}

main();
```

Run normally (interpreter):

```bash
resilient safe_div.rs
# 100 / 7 = 14
```

Run with the audit pass to see what the verifier proved at
compile time vs left as runtime checks:

```bash
resilient --audit safe_div.rs
```

With `--features z3`, the `b != 0` precondition becomes a
discharged proof obligation rather than a runtime check on
every call.

## Pick a backend

Resilient ships three execution backends. All accept the same
source — pick based on workload shape:

```bash
# Tree walker — fast to start, slow per-instruction.
# Best for one-shot scripts and during dev/test.
resilient prog.rs

# Bytecode VM — ~12× faster than the tree walker on fib(25).
# Best for medium workloads where you want fast startup AND
# decent throughput.
resilient --vm prog.rs

# Cranelift JIT — ~12× faster than the VM. Compile time is
# real, so use this for hot loops and long-running programs
# where compile cost amortizes.
cargo build --release --features jit
resilient --jit prog.rs
```

See the [performance page](performance) for the actual numbers
and the JIT's current limitations (no closures/structs/while
loops yet — interp/VM accept all of those).

## Use cases

### Embedded / safety-critical

This is what Resilient was built for. The sibling
[`resilient-runtime`](no-std) crate is `#![no_std]`-friendly
and cross-compiles to `thumbv7em-none-eabihf` (Cortex-M4F class
MCU). The `Value` enum carries `Int`, `Bool`, and (with
`--features alloc`) `Float`, `String`. The host build uses the
full interpreter + VM; the embedded build uses just the runtime
value layer + ops.

The `live { }` block is the headline feature for this
target — when a sensor read returns a corrupt frame, an I2C
transaction times out, or a divide-by-zero would crash, the
runtime restores the block's state and re-runs it instead of
panicking.

### Verified utilities

Use the contract layer (`requires` / `ensures`) for code where
"it works on the inputs we tested" isn't good enough. Z3 backs
the verifier (`--features z3`); proofs that succeed don't emit
runtime checks. For compliance / certification:
`--emit-certificate ./certs prog.rs` writes one SMT-LIB2 file
per discharged obligation, each independently re-verifiable
under any compatible solver.

```bash
cargo run --features z3 -- --emit-certificate ./certs examples/cert_demo.rs
z3 -smt2 ./certs/ident_round__decl__0.smt2
# unsat   ← the proof: negation is unsatisfiable, so the original holds
```

### Bytecode-VM scripting

If you don't need verification or live-block recovery and
just want a fast, simple scripting language, the bytecode VM
is competitive with Python on tight numeric loops while
giving you a real type system and contracts when you want
them.

### JIT hot loops

The Cranelift JIT (Phases B–H) compiles arithmetic + control
flow + recursion + function calls + let bindings to native
code. On `fib(25)` it's within ~1.4× of native Rust. For
loop-heavy or recursion-heavy workloads, this is the right
backend.

What the JIT doesn't yet compile: reassignment (`x = x + 1`),
while loops, closures, structs, arrays, `live { }` blocks.
Use the VM for those (it accepts all of them) until the
relevant JIT phase ships.

## REPL

```bash
resilient
> let x = 5;
> x + 10
15
> :examples       # show a few canned examples
> :exit
```

The REPL also accepts `:typecheck` to toggle static type
checking.

## Editor integration

Run the LSP server (built with `--features lsp`):

```bash
resilient --lsp
```

For Neovim with `nvim-lspconfig`, see
[LSP.md](https://github.com/EricSpencer00/Resilient/blob/main/LSP.md)
for the config snippet. VS Code needs a thin extension that
points at the binary with `--lsp`.

The server publishes `did_open` + `did_change` diagnostics
today. Hover, completion, and go-to-definition are planned
but not shipped.

## Where next?

- [Design Philosophy](philosophy) — why the language looks the way it does
- [Syntax Reference](syntax) — the full grammar in one page
- [Performance](performance) — the bench numbers and methodology
- [no_std Runtime](no-std) — embedding on a Cortex-M
