# Resilient

A programming language designed for extreme reliability in embedded and safety-critical systems.

📖 **Docs**: [ericspencer.us/Resilient](https://ericspencer.us/Resilient/)
&nbsp;·&nbsp; [Onboarding](https://ericspencer00.github.io/Resilient/getting-started)
&nbsp;·&nbsp; [Design Philosophy](https://ericspencer00.github.io/Resilient/philosophy)
&nbsp;·&nbsp; [Performance](https://ericspencer00.github.io/Resilient/performance)

## Core Philosophy

Resilient is a statically-typed, compiled programming language designed from the ground up for extreme reliability in embedded and safety-critical systems. Its core philosophy is built on three pillars:

### Resilience
Failures are inevitable. Resilient treats them as expected events, not exceptions. The language provides built-in mechanisms for code to "self-heal" and continue execution, ensuring the system never enters a non-functional state.

### Verifiability
It shouldn't just work; it must be provably correct. Resilient integrates concepts from formal methods to allow developers to define and verify system invariants at compile time.

### Simplicity
The syntax is designed to be minimal and unambiguous, reducing the cognitive load on the developer and minimizing the surface area for bugs.

## Target Applications

- **Automotive**: Engine control units (ECUs), autonomous driving systems, braking systems.
- **Aerospace**: Flight control systems, drone autopilots.
- **Industrial Automation**: Robotic arms, safety controllers on manufacturing lines.
- **Medical Devices**: Infusion pumps, monitoring equipment.

## Key Features

### The "Live" Block: Self-Healing Code

The cornerstone of Resilient is the live block. Any code within a live block is supervised by the Resilient runtime. If a recoverable error occurs within this block, the runtime will not panic or halt. Instead, it will reset the state of the block to its last known-good state and re-execute it.

```
// Example of a live block that handles division by zero
live {
    let result = 100 / user_input;
    println("Result: " + result);
}
```

### Formal Methods Lite: Invariants with assert

For the MVP, we introduce a simple form of formal verification using an enhanced assert macro. These assertions define critical system invariants—conditions that must always be true.

```
// System invariant that must not be violated
assert(fuel_level >= 0, "Fuel level cannot be negative");
```

### Static Type Checking

Resilient includes static type checking to catch type errors before runtime. This helps prevent common errors and ensures more reliable code.

```
// Function with typed parameters
fn calculate_velocity(float distance, float time) {
    return distance / time;
}
```

### Improved Error Handling

Resilient provides detailed error messages and has sophisticated error recovery mechanisms, especially within live blocks.

## Getting Started

### Running the REPL

```bash
cd resilient
cargo run
```

### Running an Example

```bash
cd resilient
cargo run -- examples/sensor_monitor.rs

# With static type checking
cargo run -- --typecheck examples/sensor_monitor.rs

# With verification audit (shows static-vs-runtime contract coverage)
cargo run -- --audit examples/sensor_monitor.rs
```

### SMT-backed verification (optional)

Resilient ships with a hand-rolled contract verifier that handles
constant folding, let-binding propagation, control-flow assumptions,
and inter-procedural chaining. For contracts beyond that subset
(e.g. universal tautologies like `x + 0 == x`), build with the
optional `z3` feature to get full SMT-backed proofs:

```bash
# macOS:  brew install z3
# Linux:  sudo apt-get install libz3-dev z3
cargo run --features z3 -- --audit prog.rs
```

The audit report tags clauses proven by Z3 separately so users can
see what the SMT layer added.

### Verification certificates (RES-071)

Once Z3 has discharged a contract obligation, you can ask the driver
to dump the proof as an SMT-LIB2 file so a downstream consumer can
re-verify it under their own solver — without trusting the Resilient
binary:

```bash
cargo run --features z3 -- --emit-certificate ./certs examples/cert_demo.rs
```

One file is written per discharged obligation:
`./certs/<fn_name>__<kind>__<idx>.smt2`. Each file is self-contained
(declares every free variable, pins call-site bindings, asserts the
negated goal, ends with `(check-sat)`). Feed it to stock Z3:

```bash
z3 -smt2 ./certs/ident_round__decl__0.smt2
# unsat        ← the proof: negation is unsatisfiable, so the original holds
```

Implies `--typecheck`. Without `--features z3`, no certificates are
emitted (the cheap folder isn't asked to produce them).

### Embedded runtime (RES-075 + RES-097 + RES-098)

The sibling `resilient-runtime/` crate carves out the value layer
+ core ops in a `#![no_std]`-compatible form, ready for a
Cortex-M class MCU. RES-075 Phase A shipped the alloc-free
`Value::Int`/`Value::Bool` types; RES-097 verified the
cross-compile; RES-098 added an opt-in `alloc` feature for
`Value::Float` (always available — stack-only) and `Value::String`
(behind the feature).

```bash
# Host build, default features (alloc-free) — 11 unit tests
cd resilient-runtime
cargo build
cargo test

# Host build with alloc — 14 unit tests (adds Float + String coverage)
cargo build --features alloc
cargo test  --features alloc
```

#### Verified cross-compile

`resilient-runtime` builds for the `thumbv7em-none-eabihf` target
(Cortex-M4F class MCU) in both feature configs:

```bash
rustup target add thumbv7em-none-eabihf

# Default (alloc-free) — Cortex-M4F has native i64 instruction
# support, no compiler_builtins shim needed.
cargo build --target thumbv7em-none-eabihf
cargo clippy --target thumbv7em-none-eabihf -- -D warnings

# With --features alloc, embedded-alloc 0.5 is pulled in.
# The lib does NOT pick a #[global_allocator] — that's the
# binary's responsibility (LlffHeap from embedded-alloc is the
# common choice for Cortex-M).
cargo build --target thumbv7em-none-eabihf --features alloc
cargo clippy --target thumbv7em-none-eabihf --features alloc -- -D warnings
```

Embedded users wire the allocator in their binary's `main()`:

```rust
use embedded_alloc::LlffHeap as Heap;
#[global_allocator]
static HEAP: Heap = Heap::empty();

fn main() -> ! {
    // initialize HEAP with a fixed-size memory pool, then use
    // resilient_runtime::Value::String / Float freely.
    loop {}
}
```

The `resilient/` crate is unaffected — it stays a single-crate
project; `resilient-runtime/` is a separate Cargo project alongside
it. A future ticket can promote both to a workspace if there's a
real reason (shared profile config, cross-crate testing).

See [`resilient-runtime-cortex-m-demo/`](resilient-runtime-cortex-m-demo/)
for a buildable example that links the runtime with an
`embedded-alloc` global allocator on Cortex-M4F (RES-101). Run
`scripts/build_cortex_m_demo.sh` from the repo root to verify the
cross-compile; the demo is a build check, not a runtime
demonstration.

#### RISC-V rv32imac (RES-176)

The runtime also cross-compiles to
`riscv32imac-unknown-none-elf` — the baseline ISA for HiFive,
GD32V, and ESP32-C3 class chips. Both the default (alloc-free)
and `alloc` feature sets build clean, and `embedded-alloc`'s
`linked_list_allocator` backend works on RISC-V without target
overrides.

```bash
rustup target add riscv32imac-unknown-none-elf
cd resilient-runtime
cargo build --target riscv32imac-unknown-none-elf
cargo build --target riscv32imac-unknown-none-elf --features alloc
cargo clippy --target riscv32imac-unknown-none-elf -- -D warnings
```

Run `scripts/build_riscv32.sh` from the repo root to execute all
three steps in one shot. CI gates the RISC-V build via the
`embedded` workflow (`.github/workflows/embedded.yml`) alongside
the Cortex-M job.

There's no separate RISC-V demo crate yet — one embedded demo
(RES-101's Cortex-M4F) is enough to exercise the `#[global_allocator]`
wiring; adding a second target would multiply maintenance for
zero new coverage.

### Available Examples

All in `resilient/examples/`. Each ships with a `.expected.txt`
sidecar so the smoke tests can verify they still produce the
documented output.

- `hello.rs` — `println("Hello, world!");`
- `minimal.rs` — smallest working program with a top-level return
- `int_math.rs` — arithmetic + integer operators
- `sensor_monitor.rs` — `live { }` block reading a synthetic sensor
- `self_healing.rs` — recovery after a transient error inside a live block
- `nested_array_demo.rs` — multi-dimensional array indexing/assignment
- `cert_demo.rs` — minimal program whose contract Z3 can discharge,
  used by `--emit-certificate` (RES-071)
- `imports_demo/` — multi-file import resolution
- `file_io_demo.rs` — round-trip through `file_read` / `file_write`
  (RES-143)

**Safety considerations for `file_read` / `file_write` (RES-143):**
the CLI has ambient filesystem authority and these builtins inherit
it with no sandboxing. Run untrusted Resilient programs inside a
chroot or container if you care about what they can touch. The
`resilient-runtime` sibling crate (used by embedded targets) has no
builtins table and therefore no file I/O surface — it stays
no_std-clean.

### Debugging

`--dump-tokens <file>` prints the lexer's token stream — one
`line:col Kind("lexeme")` per line, ending with `Eof` — and
exits. Useful when a parser error points at a mystery token and
you want to see what the scanner actually emitted, without
editing source (RES-112). Mutually exclusive with `--lsp`.

```sh
resilient --dump-tokens examples/hello.rs
# 2:1  Function("fn")
# 2:4  Identifier("main")("main")
# 2:8  LeftParen("(")
# ...
# 6:1  Eof("")
```

`--dump-chunks <file>` compiles the program to VM bytecode and
prints a human-readable disassembly of every chunk — `main` plus
each user function — with constants, per-op offset/line/opname
columns, and absolute jump targets (RES-173). The output reflects
the RES-172 peephole pass, so what you see is what runs.

```sh
resilient --dump-chunks examples/hello.rs
# === main ===
# constants:
#   const[0] = "Hello, Resilient world!"
# code:
#   0000  L2   Const 0      ; const[0] = "Hello, Resilient world!"
#   0001  L2   Call 0       ; -> println
#   0002  L2   Return
```

Mutually exclusive with `--dump-tokens` and `--lsp`. The output
format is stable — external tools are welcome to parse it; the
disassembler module comment documents the exact column contract.

### REPL Commands

- `help` - Show help message
- `exit` - Exit the REPL
- `clear` - Clear the screen
- `examples` - Show example code snippets
- `typecheck` - Toggle type checking

### Randomness (RES-150)

The `random_int(lo, hi)` and `random_float()` builtins are backed
by **SplitMix64**, a tiny deterministic PRNG. The seed is either
pinned with `--seed <u64>` (for reproducible runs) or derived from
the monotonic clock at startup and echoed to stderr as
`seed=<N>` so a failing run can be replayed verbatim.

**These are NOT cryptographic.** Do not use `random_*` for key
material, session tokens, salts, nonces, or anything an attacker
could exploit if guessed. SplitMix64 is chosen for determinism and
small code size (≈15 LOC, zero dependencies), not unpredictability.
When the language grows a cryptographic-grade primitive it will
live under a separate name with the appropriate guarantees.

## Syntax Requirements

See the [SYNTAX.md](SYNTAX.md) file for detailed syntax requirements and examples. Key points:

- Function parameters carry explicit types; zero-parameter functions use `fn name()`
- Static variables maintain state between function calls
- Live blocks provide self-healing capabilities
- Assertions validate system invariants

## Project Status

Active development happens one ticket at a time. See [.board/ROADMAP.md](.board/ROADMAP.md)
for the goalpost ladder and [.board/tickets/DONE/](.board/tickets/DONE/) for
the full ledger. Each commit of the form `RES-NNN: summary` closes one ticket.

### What works today

- Functions (with and without parameters), forward references
- `let` and `static let` bindings, reassignment
- Arithmetic, comparison, logical, bitwise, and shift operators
- Prefix `!` and `-`
- Hex (`0xFF`) and binary (`0b1010`) integer literals with `_` separators
- Block `/* */` and line `//` comments
- `if` / `else`, `while` (with runaway guard)
- `live { }` self-healing blocks with retry
- `assert(cond, msg)` with operand values in the error
- Built-ins: `println`, `print`, `len`, `abs`, `min`, `max`, `sqrt`,
  `pow`, `floor`, `ceil`
- Clean `line:col:` error diagnostics
- 50+ passing tests covering lexer, parser, typechecker, interpreter,
  and example programs (golden file sidecars in `resilient/examples/`)
- Zero panic paths in the parser or lexer — every error is recoverable

### Performance (RES-106)

A representative workload: fib(25), 242,785 recursive calls,
on Apple M1 Max. See `benchmarks/RESULTS.md` for the full
table including Python/Node/Lua/Ruby and the bench scripts.

| backend                  | fib(25) median | vs interp |
|--------------------------|----------------|-----------|
| Resilient (interp)       | 406.7 ms       | 1×        |
| Resilient (VM, RES-082)  | 33.7 ms        | 12×       |
| Resilient (JIT, RES-106) | **2.8 ms**     | **145×**  |
| Rust (native -O)         | 2.0 ms         | 204×      |

The Cranelift JIT (`--features jit --jit`) is **~12× faster
than the bytecode VM** and within **~1.4×** of native Rust on
this workload, beating Lua (7.1 ms), Python 3 (32.5 ms),
Node.js (62.8 ms), and Ruby (71.2 ms). Compile time is
included in the measurement (amortized across the ~242k
calls); for one-shot arithmetic the VM is the right backend.

### What's next

- G4 (full source spans with snippets / carets)
- G5 (replace hand-rolled lexer with `logos`)
- G6 (one canonical AST, retire the unwired `parser.rs`)
- G7 (real type checker: inference, unification, exhaustiveness)
- G8–G10 (function contracts, symbolic assert, live-block invariants)
- G11+ (stdlib, structs, pattern matching, cranelift backend, LSP,
  `no_std`, self-hosting)
