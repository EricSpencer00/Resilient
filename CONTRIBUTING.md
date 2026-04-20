# Contributing to Resilient

Welcome — and thank you for your interest in Resilient! Contributions from
humans, AI agents, and automated tooling are all equally welcome. Every
improvement, no matter how small, helps push the language forward.

---

## Setting Up the Development Environment

### Prerequisites

- **Rust** (stable toolchain) — install via [rustup.rs](https://rustup.rs/)
- **z3** (optional, for SMT-backed verification)
  - macOS: `brew install z3`
  - Linux: `sudo apt-get install libz3-dev z3`

### Building

```bash
# Clone the repo
git clone https://github.com/EricSpencer00/Resilient.git
cd Resilient

# Build the compiler
cd resilient
cargo build

# Build the embedded runtime
cd ../resilient-runtime
cargo build

# Build with the alloc feature
cargo build --features alloc

# Build with z3 support (requires z3 installed)
cd ../resilient
cargo build --features z3
```

### Running Tests

```bash
# Compiler + interpreter tests
cd resilient
cargo test

# With Z3 integration tests
cargo test --features z3

# Embedded runtime — default (alloc-free, 11 tests)
cd resilient-runtime
cargo test

# With alloc feature (14 tests)
cargo test --features alloc

# With static-only feature (13 tests)
cargo test --features static-only
```

### Cross-compile targets (optional)

```bash
rustup target add thumbv7em-none-eabihf   # Cortex-M4F
rustup target add thumbv6m-none-eabi      # Cortex-M0/M0+
rustup target add riscv32imac-unknown-none-elf

cd resilient-runtime
cargo build --target thumbv7em-none-eabihf
cargo build --target thumbv6m-none-eabi
cargo build --target riscv32imac-unknown-none-elf
```

---

## Project Structure

```
Resilient/
├── resilient/                      # Compiler, REPL, and CLI driver
│   ├── src/                        # Lexer, parser, type checker, VM, JIT, builtins
│   └── examples/                   # Example programs (each with .expected.txt sidecar)
├── resilient-runtime/              # no_std-compatible embedded runtime crate
│   └── src/                        # Value types, ops, sink abstraction
├── resilient-runtime-cortex-m-demo/# Cortex-M4F cross-compile demo (build check)
├── docs/                           # Static site source (published to GitHub Pages)
├── benchmarks/                     # Benchmark scripts and RESULTS.md
├── self-host/                      # Self-hosting bootstrap experiments
├── scripts/                        # Helper shell scripts (CI, size gate, etc.)
├── .board/                         # Project management board
│   ├── ROADMAP.md                  # Goalpost ladder (G1–G20+)
│   └── tickets/                    # Ticket files (OPEN / IN_PROGRESS / DONE)
└── .github/workflows/              # CI workflow definitions
```

---

## Ticket and Issue Workflow

Resilient uses a lightweight file-based ticket system under `.board/tickets/`.
GitHub Issues mirror the OPEN tickets so external contributors can see and
claim work without needing special repo access.

### Ticket lifecycle

```
OPEN  →  IN_PROGRESS  →  DONE
```

- **OPEN** — `/.board/tickets/OPEN/RES-NNN-short-title.md`
  Filed when work is defined but not yet started.
- **IN_PROGRESS** — `/.board/tickets/IN_PROGRESS/RES-NNN-short-title.md`
  Move the file when you start work. Add your name / agent ID to the ticket.
- **DONE** — `/.board/tickets/DONE/RES-NNN-short-title.md`
  Move the file when the work lands on `main`. Record the closing commit hash.

### Claiming a ticket

1. Pick an OPEN ticket (or open a GitHub Issue for new work).
2. Move the ticket file from `OPEN/` to `IN_PROGRESS/`.
3. Add a `Claimed-by:` line to the ticket header.
4. Open a draft PR referencing the ticket number early so others know work is
   in progress.

---

## Commit Format

- **Ticket work**: `RES-NNN: short description`
  Example: `RES-207: add struct literal syntax to parser`
- **Other fixes / chores**: free-form, but keep the first line under 72 chars.
  Example: `Fix typo in CONTRIBUTING.md`
- **Multi-line bodies** are welcome for complex changes — explain *why*, not
  just *what*.

---

## Coding Standards

### General

- Follow existing code style. Run `cargo fmt` before committing.
- Run `cargo clippy -- -D warnings` and address all warnings.
- Every new language feature must have tests. Every new example program must
  have a `.expected.txt` golden-output sidecar in `resilient/examples/`.

### `resilient-runtime/`

- Must remain `#![no_std]` compatible in the default feature set.
- **Zero use of `std` types** (no `Vec`, `String`, `Box`, etc.) outside of
  `#[cfg(feature = "alloc")]` gates.
- **Zero panics**: use `Result` / `Option` and propagate errors. Every
  `unwrap()` or `expect()` is a bug.

### `resilient/` (compiler / CLI)

- **Zero panics in the parser and lexer.** Every error path must return a
  typed `Error` and be surfaced as a clean diagnostic. A panic is a bug.
- Diagnostics carry `line:col:` source positions. Don't add bare `println!`
  or `eprintln!` debug output — use the existing diagnostic infrastructure.
- New built-in functions go in the builtins table with a doc-comment and a
  test.

### Tests

- Unit tests live next to the code they test (`#[cfg(test)]` modules).
- Integration / example tests use the golden `.expected.txt` sidecar pattern:
  run `cargo run -- examples/foo.rs` and diff stdout against
  `examples/foo.expected.txt`.
- Tests that require z3 must be gated with `#[cfg(feature = "z3")]` or placed
  under `cargo test --features z3`.

---

## Pull Request Guidelines

- **Keep PRs small and focused.** One ticket = one PR is ideal.
- **Link the GitHub Issue** in the PR description:
  `Closes #<issue-number>` or `Fixes #<issue-number>`.
- **CI must be green** before requesting review. The workflows gate on:
  - `cargo test` (host)
  - `cargo test --features z3`
  - `cargo test --features alloc` in `resilient-runtime`
  - Cross-compile for `thumbv7em-none-eabihf`, `thumbv6m-none-eabi`,
    and `riscv32imac-unknown-none-elf`
  - Size gate (`.text` ≤ 64 KiB for the Cortex-M4F demo)
  - Performance gate (`cargo bench` regression check)
- Include a brief description of *what* changed and *why*.
- Update `CHANGELOG` or the relevant ticket file if appropriate.

---

## For AI Agent Contributors

AI agents are first-class contributors. The same rules apply as for humans:

- Follow the commit format (`RES-NNN: short description` for ticket work).
- Open PRs against `main`; do not force-push.
- Keep PRs focused — one ticket, one concern.
- All CI checks must pass before requesting a merge.
- If an agent opens a PR on behalf of a human, include a `Co-Authored-By:`
  trailer in the commit message so authorship is transparent.
- Agents may claim tickets by moving them from `OPEN/` to `IN_PROGRESS/` and
  adding `Claimed-by: <agent-id>` to the ticket header.

Example commit trailer for agent-assisted work:

```
Co-Authored-By: Claude Sonnet <noreply@anthropic.com>
```

---

## Code of Conduct

This project follows the [Contributor Covenant Code of Conduct](CODE_OF_CONDUCT.md).
By participating you agree to uphold it. Please report unacceptable behavior to
[ericspencer1450@gmail.com](mailto:ericspencer1450@gmail.com).
