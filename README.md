# Resilient

A programming language designed for extreme reliability in embedded and safety-critical systems.

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)
[![Contributions Welcome](https://img.shields.io/badge/contributions-welcome-brightgreen.svg)](CONTRIBUTING.md)
[![GitHub Issues](https://img.shields.io/github/issues/EricSpencer00/Resilient)](https://github.com/EricSpencer00/Resilient/issues)

**Resilient is open source under the MIT license — contributions from humans and AI agents are welcome.**

📖 **Docs**: [ericspencer.us/Resilient](https://ericspencer.us/Resilient/)
&nbsp;·&nbsp; [Onboarding](https://ericspencer00.github.io/Resilient/getting-started)
&nbsp;·&nbsp; [Design Philosophy](https://ericspencer00.github.io/Resilient/philosophy)
&nbsp;·&nbsp; [Performance](https://ericspencer00.github.io/Resilient/performance)
&nbsp;·&nbsp; [Memory Model](https://ericspencer00.github.io/Resilient/memory-model)
&nbsp;·&nbsp; [Contributing](CONTRIBUTING.md)

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

## Safety Standards

Resilient is not a certified tool and does not claim DO-178C,
ISO 26262, or IEC 61508 conformance — tool qualification is a
multi-year effort that has not started. What the language does
provide is a set of features (formal contracts, re-verifiable
SMT-LIB2 certificates, signed manifests, `static-only` heap
enforcement, ASCII-only identifiers, deterministic execution)
that map directly to specific objectives in each standard and
reduce the evidence burden on the integrator. See the
[Certification and Safety Standards](https://ericspencer00.github.io/Resilient/certification)
page for the concrete objective-by-objective mapping and the
honest list of gaps.

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

### Docker (RES-203)

A prebuilt image is published to GitHub Container Registry on
every tagged release. Pull + run without installing Rust:

```bash
docker run --rm ghcr.io/ericspencer00/resilient:latest --help

# Run a source file by mounting it in:
docker run --rm -v "$PWD":/work -w /work \
    ghcr.io/ericspencer00/resilient:latest examples/hello.rs
```

The image is multi-arch (linux/amd64 + linux/arm64), built from
`Dockerfile` at repo root. Ships with the `--features z3` build
so the SMT-backed verifier works out of the box. Runs as an
unprivileged `resilient` user (UID 1001) — mount working
directories with matching permissions if you want the binary to
write certificates / artifacts back.

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

#### Signed certificates (RES-194)

Pass `--sign-cert <path-to-ed25519-private-key.pem>` alongside
`--emit-certificate <dir>` to write a 64-byte Ed25519 signature to
`<dir>/cert.sig`. The signed payload is the byte-for-byte
concatenation of the `.smt2` files in the directory (sorted by
filename, joined with `\n`); the signature binds the certificate
set to the signer's key.

```bash
# Sign during emit:
resilient -t --emit-certificate ./certs --sign-cert ~/.resilient-priv.pem src/main.rs

# Verify against the binary's embedded public key:
resilient verify-cert ./certs

# Or against a custom public key (e.g. a rotated / test key):
resilient verify-cert ./certs --pubkey ./trusted-pub.pem
```

Exit codes from `verify-cert`: `0` = valid signature, `1` =
tampered / wrong key, `2` = usage error (missing directory, etc.).

The committed public key lives at `resilient/src/cert_key.pem` and
is baked into the binary via `include_str!`. Key management (the
corresponding private key + rotation) is a human concern — the
signing key is NOT committed. See the ticket's Notes for the
rotation-follow-up plan.

The PEM format is a minimal `-----BEGIN ED25519 {PUBLIC,PRIVATE}
KEY-----` envelope around 64 hex chars (32 raw bytes). A tiny
helper is provided inside the `cert_sign` module; external tools
that want to generate a compatible keypair can use any Ed25519
library and format the output identically.

#### Certificate manifest (RES-195)

Every `--emit-certificate <dir>` run also writes a
`manifest.json` index:

```json
{
  "program": "fib.rs",
  "obligations": [
    {
      "fn": "fib",
      "kind": "ensures",
      "idx": 0,
      "cert": "fib__ensures__0.smt2",
      "sha256": "<64 hex>",
      "sig": "<128 hex>"
    }
  ]
}
```

- `sha256` is always present and always over the `.smt2` file's
  bytes — consumers can detect tampering without holding any
  cryptographic key.
- `sig` is present only when `--sign-cert` was passed; it's an
  Ed25519 signature over THIS cert's bytes (not the batch). The
  top-level `cert.sig` from RES-194 still covers the whole
  payload — both are written on signed runs so either can be
  used for verification.

The `verify-all` subcommand re-checks every obligation:

```bash
# Fast cryptographic-only pass:
resilient verify-all ./certs

# With a custom public key (for rotated / test keys):
resilient verify-all ./certs --pubkey ./trusted-pub.pem

# Extra: re-run Z3 on each cert (if the `z3` binary is on PATH):
resilient verify-all ./certs --z3
```

Output is a one-row-per-obligation table with `sha256`/`sig`/`z3`
columns (`ok`, `FAIL`, `-` = skipped). Exit 0 iff every checked
cell passes; exit 1 on any failure or missing file; exit 2 on
usage errors. Without `--z3`, the column is skipped — the
cryptographic checks alone are a strong regression signal.

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

#### Sink abstraction for `println` (RES-180)

The runtime exposes a tiny `sink::Sink` trait — one method,
`write_str(&mut self, s: &str) -> Result<(), SinkErr>` — plus
a global `sink::println` / `sink::print` routed through the
currently-installed sink. Users wire a sink once at program
start with `sink::set_sink(&mut their_sink)`; subsequent
`println` calls thread text through it. On embedded this is
typically a UART / semihosting / ring-buffer writer; for tests
a memory-backed `BufSink` captures output for assertions.

Optional `std-sink` feature exports a `StdoutSink` convenience
type that forwards to `std::io::stdout()` for users who want
the old std-host behavior without rolling their own:

```rust
use resilient_runtime::sink::{set_sink, StdoutSink};
static mut STDOUT_SINK: StdoutSink = StdoutSink;
fn main() {
    unsafe { set_sink(&mut STDOUT_SINK); }
    resilient_runtime::sink::println("hello from the runtime").unwrap();
}
```

The core Sink / print / println surface is always available
— no feature flag needed. `StdoutSink` sits behind `std-sink`
because it pulls in `std`, which is incompatible with
`no_std` embedded deployments.

Thread-safety: the global sink pointer is held in an
`UnsafeCell` wrapped in a `Sync` newtype. Sound for embedded
bare-metal (single-core / single-thread). Tests serialize
via a shared `SINK_TEST_LOCK` (same pattern as RES-150's RNG
lock). Multi-threaded embedded use would need a
`critical-section` crate or a `spin::Mutex` — left for a
follow-up when the use case appears.

#### Code-size budget (RES-179)

CI runs `cargo bloat` against the release Cortex-M4F demo on
every push + PR via the `size-gate` workflow. The gate fails
if the `.text` section exceeds **64 KiB** (overridable via the
`SIZE_BUDGET_KIB` env var in `.github/workflows/size_gate.yml`).

Current measurement (release, `thumbv7em-none-eabihf`):

| Section | Size       | % of budget |
| ------- | ---------- | ----------- |
| `.text` | **2.3 KiB** (~2 355 bytes) | **3.6 %** |

Plenty of headroom — the budget is deliberately generous per the
ticket Notes; tighten in a follow-up once we have a stable
baseline across releases. The top .text contributors today are
`compiler_builtins::mem::memcpy` (~31 %), the demo's own `main`
(~21 %), and `embedded_alloc`'s `dealloc` / `alloc` wrappers
(~12 % / ~11 % respectively) — everything else is single-digit
bytes.

Run locally:

```bash
scripts/check_size_budget.sh                     # default 64 KiB
SIZE_BUDGET_KIB=32 scripts/check_size_budget.sh  # tighter budget
```

The script prints `cargo bloat`'s top-20 symbol table either
way, so a failing run attributes the regression without needing
to re-run anything.

#### `--features static-only` (RES-178)

For safety-critical projects that forbid dynamic allocation
entirely, the runtime crate exposes a mutually-exclusive
`static-only` feature. Under it, the reduced `Value` surface is:

| feature flags             | Value::Int | Value::Bool | Value::Float | Value::String |
| ------------------------- | :--------: | :---------: | :----------: | :-----------: |
| (default)                 | ✅         | ✅          | ✅           | ✕             |
| `--features alloc`        | ✅         | ✅          | ✅           | ✅            |
| `--features static-only`  | ✅         | ✅          | ✅           | ✕             |
| `alloc + static-only`     | *build fails: `compile_error!`* |

`static-only` is an assertion, not an enabler: setting it makes
any attempt to add a heap-bearing Value variant without
feature-gating it fail the build. The existing
`#[cfg(feature = "alloc")]` gates around `Value::String` (and
their future cousins for Array/Map when those land here) already
provide structural enforcement; `static-only` adds the intent
contract + the explicit mutex.

```bash
cd resilient-runtime
# Alloc-free build (same as default — feature is assertive).
cargo build --features static-only
cargo test  --features static-only     # 13 tests
# Prove the mutex:
cargo build --features "alloc static-only"   # compile_error!
```

The `resilient/` CLI crate is NOT built with `static-only` — the
compiler itself needs alloc. The feature is runtime-only.

#### Cortex-M0 / M0+ / M1 thumbv6m (RES-177)

The Armv6-M lower-end (Cortex-M0 class, e.g. RP2040's dual-M0+
cores or STM32F0) is the watch target for "did anything pull
in a dep that assumes M4F-only features?" — no FPU, no 32-bit
atomics, no DSP extensions. The runtime today uses **no
atomics** anywhere (RES-141's telemetry counters live in the
`resilient` CLI binary, not in `resilient-runtime`), so the
ticket's `#[cfg(target_has_atomic = "32")]` gating isn't
triggered today; the CI gate will catch any future regression
the moment a dep tries to pull in `AtomicU32`.

```bash
rustup target add thumbv6m-none-eabi
cd resilient-runtime
cargo build --target thumbv6m-none-eabi
cargo build --target thumbv6m-none-eabi --features alloc
cargo clippy --target thumbv6m-none-eabi -- -D warnings
```

`scripts/build_cortex_m0.sh` is the one-shot equivalent. The
`alloc` feature builds clean (embedded-alloc + linked_list_allocator
+ critical-section's M0 single-core backend all link). `Value::Float(f64)`
also compiles (soft-float via libgcc), but floats are slow on M0
— the ISA has no FPU, so every float op is a runtime library call.
Avoid floats on M0 when you can; stick to `Value::Int(i64)`.

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

### Formatter

`resilient fmt <file>` pretty-prints a Resilient source file in
canonical style (4-space indent, brace-on-same-line, contracts
indented under the function signature). By default it prints to
stdout; pass `--in-place` to overwrite the file.

```bash
resilient fmt examples/hello.rs              # print to stdout
resilient fmt --in-place src/main.rs         # overwrite
```

The formatter refuses to touch input with parse errors (exits 1).
It is a structural round-trip — comments are not preserved today;
only run it on code you're willing to re-attach comments to by
hand. See [Tooling Reference](https://ericspencer00.github.io/Resilient/tooling#formatter)
for the full contract.

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

## Performance

Run `cargo bench --manifest-path resilient/Cargo.toml` to benchmark the
tree-walker interpreter on three representative workloads (recursive
`fib(25)`, bubble-sort, and string concatenation). Results are saved to
`resilient/target/criterion/`; a captured baseline lives at
[`benchmarks/baseline.txt`](benchmarks/baseline.txt).

## Syntax Requirements

See the [SYNTAX.md](SYNTAX.md) file for detailed syntax requirements and examples. Key points:

- Function parameters carry explicit types; zero-parameter functions use `fn name()`
- Static variables maintain state between function calls
- Live blocks provide self-healing capabilities
- Assertions validate system invariants

## Project Status

Active development happens one ticket at a time. See [ROADMAP.md](ROADMAP.md)
for the goalpost ladder and [GitHub Issues (closed)](https://github.com/EricSpencer00/Resilient/issues?q=is%3Aissue+is%3Aclosed) for
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

### Self-hosting progress (G20)

G20 is a long arc. The first milestone is a Resilient program
that can lex Resilient source.

- **RES-196** — [`self-host/lex.rs`](./self-host/lex.rs): a byte-
  level lexer for a restricted subset of the language, written
  in Resilient itself. Recognizes identifiers, integer + string
  literals, the `fn` / `let` / `return` / `if` / `else` / `while` /
  `true` / `false` keywords, single-char punctuation,
  single-char operators, the two-char comparison / logical
  operators, and `//` line comments. Whitespace-skipping with
  line / column tracking.

  Run it: `./self-host/run.sh` (diffs output against
  [`self-host/hello.tokens.txt`](./self-host/hello.tokens.txt)).

  Not in CI — informative only until the self-hosted toolchain
  becomes load-bearing. See the source file's top-comment for
  the feature gaps (multiline strings, block comments, `live`
  contracts, float / bytes literals) and the parser workarounds
  the prototype needed to land today.
