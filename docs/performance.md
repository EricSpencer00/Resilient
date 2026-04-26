---
title: Performance
nav_order: 6
permalink: /performance
---

# Performance
{: .no_toc }

How fast is each backend? Honest measurements with full
methodology so you can reproduce.
{: .fs-6 .fw-300 }

<details open markdown="block">
  <summary>Table of contents</summary>
  {: .text-delta }
- TOC
{:toc}
</details>

---

## Headline numbers

`fib(25)` on Apple M1 Max, `hyperfine --warmup 2 --runs 5`:

| backend                  | median   | vs interp | vs native |
|--------------------------|----------|-----------|-----------|
| Resilient (interp)       | 406.7 ms | 1×        | 0.005×    |
| Resilient (VM)           | 33.7 ms  | 12×       | 0.06×     |
| **Resilient (JIT)**      | **2.8 ms** | **145×**  | **0.71×** |
| Rust (native -O)         | 2.0 ms   | 204×      | 1×        |

For context, the same workload in popular scripting languages:

| language     | median   |
|--------------|----------|
| Python 3     | 32.5 ms  |
| Node.js      | 62.8 ms  |
| Lua          | 7.1 ms   |
| Ruby         | 71.2 ms  |

The Cranelift JIT beats every scripting language in the table,
including Lua, and is essentially tied with native Rust on
this workload (within statistical noise). The bytecode VM is
competitive with Python on the same comparison.

## Methodology

```bash
# Build the binaries
cd resilient
cargo build --release             # default features (interp + VM)
cargo build --release --features jit
cp target/release/rz target/release/rz-with-jit

# Build the native baseline
rustc -O ../benchmarks/fib/fib_native.rs \
      -o ../benchmarks/fib/fib_native

# Run the bench
hyperfine --warmup 2 --runs 5 \
  "target/release/rz ../benchmarks/fib/fib.rs" \
  "target/release/rz --vm ../benchmarks/fib/fib_vm.rs" \
  "target/release/rz-with-jit --jit ../benchmarks/fib/fib_jit.rs" \
  "../benchmarks/fib/fib_native"
```

Or just run the all-in-one script:

```bash
./benchmarks/run.sh
# → benchmarks/RESULTS.md gets refreshed
```

### What's being measured

End-to-end binary invocation. That is:

- **Interpreter**: tree-walk + execute.
- **VM**: parse → AST → bytecode-compile → stack-VM execute.
- **JIT**: parse → AST → cranelift-lower → register-alloc →
  codegen → finalize → invoke compiled `main`.
- **Rust native**: pre-compiled `-O` binary, just invoke.

For the JIT, this means **compile time is included** in the
measurement. For `fib(25)` (~242,785 recursive calls), the
compile cost amortizes well — per-call cost dominates. For a
one-shot program (`return 1 + 2;`) the VM beats the JIT
because the compile time exceeds the entire execution time.

The right backend depends on the workload, not on which one
"is fastest" in the abstract.

## What the JIT can compile

As of Phase H (RES-105), the JIT lowers:

- Integer literals + boolean literals
- All four arithmetic ops: `+`, `-`, `*`, `/`, `%`
- All six comparison ops: `==`, `!=`, `<`, `<=`, `>`, `>=`
- `if` / `else` with arbitrary nesting; bare `if` + fallthrough
- `let` bindings + identifier reads (function-scoped, immutable)
- `return EXPR;`
- `fn name(int p1, int p2, ...)` declarations
- Direct `name(args)` calls including recursion + mutual recursion

What it doesn't yet do (use the VM instead):

- Reassignment (`x = x + 1`) — RES-107 (planned)
- `while` loops — RES-107 (planned)
- Closures / nested fns
- Structs, arrays, strings
- `live { }` blocks

The VM and tree walker accept all of the above.

## Why three backends?

Each backend is a strict superset of the previous in
expressiveness, and a strict subset in startup speed. Pick
based on workload:

| Workload shape                          | Best backend |
|-----------------------------------------|--------------|
| One-shot script (run once, exit)        | Tree walker  |
| Medium server / batch job               | Bytecode VM  |
| Long-running compute / hot loop / recursion | JIT      |
| Embedded MCU                            | Tree walker (uses `resilient-runtime`) |

The interpreter is canonical — when in doubt about semantics,
its behavior wins. The VM and JIT must produce identical
output for any program they both accept; the test suite
includes this as an invariant.

## Other benchmarks

`benchmarks/RESULTS.md` also covers:

- **`sum 1..100000`** — while-loop accumulator. Interp at 47
  ms; not yet measured on VM/JIT (loop bytecode + reassignment
  not in the JIT yet — see RES-107).
- **Contract overhead** — 100k `safe_div` calls with vs without
  a `requires` clause. ~18% overhead today; expected to drop to
  near-zero as more contracts are statically discharged
  (RES-068 elides runtime checks for fully-proven functions).

See the [full RESULTS.md](https://github.com/EricSpencer00/Resilient/blob/main/benchmarks/RESULTS.md)
for the raw hyperfine output.

## Hardware caveats

All numbers are Apple M1 Max. Other hardware (x86_64 server,
ARM SBC, etc.) will produce different absolute numbers but the
**ratios between backends should hold** — the VM is a constant
factor over the interpreter, the JIT is a constant factor over
the VM, and native Rust is the same constant factor over
either as the program is short-running. Re-run `benchmarks/run.sh`
on your target if absolute numbers matter.

## History

- **RES-082** measured the original VM number (32.0 ms) shortly
  after RES-076 shipped the bytecode foundation. ~12.6× over the
  tree walker.
- **RES-095** confirmed line-attributed VM diagnostics didn't
  regress the number.
- **RES-106** added the JIT row (2.8 ms): ~12× over the VM,
  ~145× over the tree walker, ~1.4× from native Rust.

The full bench history is in [closed GitHub Issues](https://github.com/EricSpencer00/Resilient/issues?q=is%3Aissue+is%3Aclosed).
