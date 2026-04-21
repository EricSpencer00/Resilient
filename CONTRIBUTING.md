# Contributing to Resilient

Welcome — and thank you for your interest in Resilient! Contributions from
humans, AI agents, and automated tooling are all equally welcome. Every
improvement, no matter how small, helps push the language forward.

New here? Skip to [Good first issues](#good-first-issues) for a handful of
tickets sized for a first PR.

---

## Quick Start

```bash
# 1. Clone
git clone https://github.com/EricSpencer00/Resilient.git
cd Resilient

# 2. Build the compiler (default features — no z3, no lsp, no jit)
cargo build --manifest-path resilient/Cargo.toml

# 3. Run the compiler test suite
cargo test --manifest-path resilient/Cargo.toml

# 4. Build the embedded runtime
cargo build --manifest-path resilient-runtime/Cargo.toml
cargo test  --manifest-path resilient-runtime/Cargo.toml

# 5. Run an example
cargo run --manifest-path resilient/Cargo.toml -- resilient/examples/hello.rs
```

If all four commands succeed, you have a working dev environment.

---

## Setting Up the Development Environment

### Prerequisites

- **Rust** (stable toolchain) — install via [rustup.rs](https://rustup.rs/).
  Edition 2024 is required; any recent stable rustc will do.
- **z3** (optional, only if you work on verifier code with `--features z3`)
  - macOS: `brew install z3`
  - Linux: `sudo apt-get install libz3-dev z3`
- **Cross-compile targets** (optional, for `resilient-runtime` cross builds):

  ```bash
  rustup target add thumbv7em-none-eabihf   # Cortex-M4F
  rustup target add thumbv6m-none-eabi      # Cortex-M0/M0+
  rustup target add riscv32imac-unknown-none-elf
  ```

### Feature flags

The `resilient` crate has several opt-in features; pick only what you need.

| Feature | Flag                     | What it enables                                                    |
|---------|--------------------------|--------------------------------------------------------------------|
| `z3`    | `cargo build --features z3`  | Z3-backed SMT verification (requires libz3).                   |
| `lsp`   | `cargo build --features lsp` | Language server over stdio (`resilient --lsp`).                |
| `jit`   | `cargo build --features jit` | Cranelift JIT backend (heavy deps; off by default).            |
| `ffi`   | `cargo build --features ffi` | Dynamic FFI via `extern "lib" { ... }` in the tree walker.     |

The `resilient-runtime` crate has its own feature set — notably
`--features alloc` (heap types) and `--features ffi-static` (static FFI
registry). See its `Cargo.toml` for the full list.

### Building

```bash
# Compiler / CLI driver
cargo build --manifest-path resilient/Cargo.toml

# With Z3 support
cargo build --manifest-path resilient/Cargo.toml --features z3

# With the LSP server
cargo build --manifest-path resilient/Cargo.toml --features lsp

# Embedded runtime
cargo build --manifest-path resilient-runtime/Cargo.toml
cargo build --manifest-path resilient-runtime/Cargo.toml --features alloc
cargo build --manifest-path resilient-runtime/Cargo.toml --features ffi-static

# Cross-compile check
cargo build --manifest-path resilient-runtime/Cargo.toml --target thumbv7em-none-eabihf
```

### Running tests

```bash
# Compiler + interpreter tests (default)
cargo test --manifest-path resilient/Cargo.toml

# With Z3 integration tests
cargo test --manifest-path resilient/Cargo.toml --features z3

# With FFI tests (tree walker)
cargo test --manifest-path resilient/Cargo.toml --features ffi

# Embedded runtime — default (no_std, alloc-free)
cargo test --manifest-path resilient-runtime/Cargo.toml

# With alloc feature
cargo test --manifest-path resilient-runtime/Cargo.toml --features alloc

# Static FFI registry
cargo test --manifest-path resilient-runtime/Cargo.toml --features ffi-static
```

### Formatting and lints

```bash
cargo fmt --all
cargo clippy --all-targets -- -D warnings
```

Both must pass before a PR can merge; CI enforces them.

---

## Project Structure

```
Resilient/
├── resilient/                      # Compiler, REPL, and CLI driver
│   ├── src/                        # Lexer, parser, type checker, VM, JIT, builtins
│   └── examples/                   # Example programs (each with .expected.txt sidecar)
├── resilient-runtime/              # no_std-compatible embedded runtime crate
│   └── src/                        # Value types, ops, sink abstraction, FFI registry
├── resilient-runtime-cortex-m-demo/# Cortex-M4F cross-compile demo (size-gated in CI)
├── docs/                           # Static site source (published to GitHub Pages)
├── benchmarks/                     # Benchmark scripts and RESULTS.md
├── self-host/                      # Self-hosting bootstrap experiments
├── scripts/                        # Helper shell scripts (CI, size gate, etc.)
├── STABILITY.md                    # Pre-1.0 stability policy (read before upgrading)
├── ROADMAP.md                      # Goalpost ladder (G1–G20+)
└── .github/workflows/              # CI workflow definitions
```

---

## The GitHub Issues Workflow

Resilient tracks all work in [GitHub Issues](https://github.com/EricSpencer00/Resilient/issues).
Each issue carries a unique `RES-NNN` identifier, a clear goal, and concrete
acceptance criteria.

### Picking up a ticket

1. Browse [open issues](https://github.com/EricSpencer00/Resilient/issues?q=is%3Aissue+is%3Aopen)
   and pick one. New contributors — look for issues tagged `good first issue`.
2. **Claim it** by commenting on the issue, then create a branch:

   ```bash
   git checkout -b res-NNN-short-title
   ```

3. Open a **draft PR** early with `Closes #N` in the body — this signals to
   others that the ticket is taken.
4. When the PR merges, the issue closes automatically via the `Closes #N` link.

### Good first issues

New contributors: the `good first issue` label on GitHub marks issues that
are well-scoped for a first PR. They come with clear acceptance criteria and
generally don't require deep knowledge of the compiler internals.

Browse them at:
<https://github.com/EricSpencer00/Resilient/issues?q=is%3Aissue+is%3Aopen+label%3A%22good+first+issue%22>

---

## Commit Format

- **Ticket work**: `RES-NNN: short description`
  Example: `RES-207: add struct literal syntax to parser`
- **Multi-ticket change**: join the ticket IDs with `/`.
  Example: `RES-209/214: stability policy doc + contributing guide overhaul`
- **Other fixes / chores**: free-form, but keep the first line under 72 chars.
  Example: `Fix typo in CONTRIBUTING.md`
- **Multi-line bodies** are welcome for complex changes — explain *why*, not
  just *what*.
- AI-agent-authored commits should include a `Co-Authored-By:` trailer so
  authorship is transparent. Example:

  ```
  Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>
  ```

---

## Coding Standards

### General

- Run `cargo fmt --all` before committing.
- Run `cargo clippy --all-targets -- -D warnings` and fix every warning —
  CI rejects clippy warnings.
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

### Stability-sensitive changes

Before changing anything listed as **stable** in [STABILITY.md](STABILITY.md),
read that file's "How Breaking Changes Land" section. Breaking changes to
stable surface require a deprecation plan in the ticket.

---

## Pull Request Checklist

Before marking a PR ready for review:

- [ ] Linked GitHub issue in the PR body (`Closes #N` / `Fixes #N`).
- [ ] Commit subject follows `RES-NNN: short description`.
- [ ] `cargo fmt --all` clean.
- [ ] `cargo clippy --all-targets -- -D warnings` clean.
- [ ] `cargo test` passes on the crates you touched (with relevant features).
- [ ] New behaviour has tests (unit or `.expected.txt` golden).
- [ ] Documentation updated if user-visible behaviour changed (README,
      SYNTAX.md, docs/, STABILITY.md CHANGELOG).
- [ ] GitHub Issue closed via `Closes #N` in the PR body.

CI gates PRs on all of the above plus:

- Cross-compile for `thumbv7em-none-eabihf`, `thumbv6m-none-eabi`, and
  `riscv32imac-unknown-none-elf`.
- Size gate (`.text` ≤ 64 KiB for the Cortex-M4F demo).
- Performance gate (`cargo bench` regression check).

Keep PRs small and focused — one ticket per PR is ideal.

---

## Releases

Releases are cut by pushing a semver tag (`vMAJOR.MINOR.PATCH`) to `main`:

```bash
git tag v0.3.0
git push origin v0.3.0
```

This triggers two workflows automatically:
- **release** — builds native binaries for Linux (amd64 + arm64) and macOS (amd64 + arm64), creates a GitHub Release with auto-generated notes and attached archives.
- **release-image** — builds and pushes a multi-arch Docker image to `ghcr.io/ericspencer00/resilient`.

Pre-releases use a tag like `v0.3.0-alpha.1` — the Docker image gets tagged but not promoted to `latest`.

---

## For AI Agent Contributors

AI agents (Claude Code, OpenHands, Codex, Devin, and others) are first-class
contributors. The same rules apply as for humans, plus a few agent-specific
notes to help you orient quickly.

### Quick Start for agents

1. **Find work** — Browse [GitHub Issues](https://github.com/EricSpencer00/Resilient/issues?q=is%3Aissue+is%3Aopen+label%3Aagent-ready)
   filtered by `agent-ready`. Each `agent-ready` issue was filed with the
   *Agent-Ready Ticket* template and includes a Goal, explicit Acceptance
   Criteria, and a list of files to touch — everything you need to start
   without further clarification.

2. **Claim it** — Comment on the issue, then create a branch:
   ```bash
   git checkout -b res-NNN-short-title
   ```

3. **Implement** — Read the issue's "Files / modules to touch" list. Run the
   acceptance criteria commands as you go.

4. **Verify before pushing**:
   ```bash
   cargo fmt --all
   cargo clippy --all-targets -- -D warnings
   cargo test --manifest-path resilient/Cargo.toml
   cargo test --manifest-path resilient-runtime/Cargo.toml
   ```

5. **Open a PR** — Target `main`. Use `Closes #N` in the body to auto-close
   the GitHub Issue.

6. **Trailer** — Include a `Co-Authored-By:` line so authorship is transparent:
   ```
   Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>
   ```

### Filing new work

If you identify a gap or improvement not already tracked, file a GitHub Issue
using the **Agent-Ready Ticket** template. Fill in Goal and Acceptance Criteria
precisely, and add the `agent-ready` label so other agents can pick it up.

### Core rules (same as humans)

- Follow the commit format (`RES-NNN: short description` for ticket work).
- Open PRs against `main`; never force-push.
- Keep PRs focused — one ticket, one concern.
- All CI checks must pass before requesting a merge.
- Include a `Co-Authored-By:` trailer on any agent-assisted commit.

---

## Code of Conduct

This project follows the [Contributor Covenant Code of Conduct](CODE_OF_CONDUCT.md).
By participating you agree to uphold it. Please report unacceptable behavior to
[ericspencer1450@gmail.com](mailto:ericspencer1450@gmail.com).
