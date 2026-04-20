# CLAUDE.md — Resilient

Guidance for Claude Code when working in this repository. These rules
override default Claude Code behaviour. Human contributor instructions
(CONTRIBUTING.md, STABILITY.md) take precedence over this file.

---

## What is this repo?

Resilient is a compiled, statically-typed language for safety-critical
embedded systems. The workspace contains:

| Crate | Purpose |
|---|---|
| `resilient/` | Compiler, CLI driver, REPL, JIT, LSP |
| `resilient-runtime/` | `#![no_std]` embedded runtime |
| `resilient-runtime-cortex-m-demo/` | Cortex-M4F cross-compile smoke test |
| `resilient-span/` | Source-span / diagnostic types |
| `benchmarks/` | Performance benchmarks |
| `fuzz/` | Fuzz harnesses |

This is an **agent-native** project — AI contributors are first-class. The
`.board/tickets/` workflow is the canonical source of work. Pick a ticket,
claim it, ship it.

---

## Quick-start commands

```bash
# Build the compiler
cargo build --manifest-path resilient/Cargo.toml

# Run all compiler tests
cargo test --manifest-path resilient/Cargo.toml

# Run runtime tests
cargo test --manifest-path resilient-runtime/Cargo.toml

# Lint (must be clean before opening a PR)
cargo clippy --all-targets -- -D warnings

# Format
cargo fmt --all
```

Optional features: `--features z3` (SMT verifier), `--features lsp`,
`--features jit`, `--features ffi`.

---

## Ticket workflow

1. Browse `.board/tickets/OPEN/` — pick any `RES-NNN-*.md` file.
2. Move it to `IN_PROGRESS/` and add `Claimed-by: Claude` to the header.
3. Open a **draft PR** early — this signals the ticket is taken.
4. When the PR is ready, move the ticket to `DONE/` and record the closing
   commit hash.

Commit format: `RES-NNN: short description` (≤72 chars on the first line).
Include a `Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>` trailer.

**Push policy: push to remote immediately after every commit.** Do not
accumulate local commits. As soon as a ticket is closed and committed, run
`git push` so the branch is on remote. Keep as little as possible local-only.

---

## Agent autonomy — what you may do freely

- Claim open tickets and move them through the board lifecycle.
- Add new source files, tests, and `.expected.txt` golden sidecars.
- Fix compiler warnings and clippy lints anywhere in the codebase.
- Add or expand documentation (README, docs/, SYNTAX.md, LSP.md).
- Update `Cargo.toml` dependency versions (patch-level only without asking).
- Open draft PRs and push to feature branches.

## Agent autonomy — STOP and ask first

- **Any change to an existing test** (unit test, integration test, or
  `.expected.txt` golden file) — see "Test protection" below.
- Changes to `unsafe` blocks — see "Security rules" below.
- Breaking changes to stable language surface (read STABILITY.md first).
- Dependency major/minor version bumps.
- Changes to `.github/workflows/` CI definitions.
- Force-pushing or amending commits that have already been reviewed.
- Anything that bypasses CI (`--no-verify`, skipping hooks).

---

## Test protection policy

**PRs that modify existing tests require maintainer approval before merge.**

This applies to:
- Any `#[cfg(test)]` module change in `resilient/` or `resilient-runtime/`.
- Any `.expected.txt` golden-output file change in `resilient/examples/`.
- Any change to fuzz harnesses in `fuzz/`.
- Any change to benchmark baselines in `benchmarks/`.

When you need to modify a test because the behaviour intentionally changed:

1. Call it out explicitly in the PR description under a **"Test changes"**
   section with a one-line rationale for each modified test.
2. Do **not** delete tests to make a PR green — fix the code instead.
3. Lowering or removing an assertion in a test is treated the same as
   deleting the test — requires the same approval.

CI will reject a PR that fails any test. A failing test is never a
reason to weaken the test; it is a reason to fix the implementation.

---

## Security rules

Resilient targets safety-critical embedded environments. Security discipline
is non-negotiable.

### No panics

- **`resilient-runtime/`**: zero panics in default (no_std) build. Use
  `Result`/`Option`. Every `unwrap()` or `expect()` is a bug.
- **`resilient/` parser and lexer**: all error paths must return a typed
  `Error` and surface a clean diagnostic. A panic in the compiler is a bug.
- Panics are acceptable only in test code and `main()` setup logic.

### `unsafe`

- Do not introduce new `unsafe` blocks without explicit justification in
  a code comment explaining the invariant that makes it sound.
- Any PR that adds or modifies `unsafe` must be flagged in the PR
  description and will require an additional reviewer.

### `no_std` constraints (`resilient-runtime/`)

- Zero use of `std` types (`Vec`, `String`, `Box`, etc.) outside of
  `#[cfg(feature = "alloc")]` gates.
- No heap allocation in the default feature set.
- Cross-compile must pass for all three embedded targets CI checks:
  `thumbv7em-none-eabihf`, `thumbv6m-none-eabi`, `riscv32imac-unknown-none-elf`.

### Supply-chain hygiene

- Do not add new dependencies without a clear reason stated in the PR.
- Prefer in-tree implementations of small utilities over new crates.
- All new crates must appear in `Cargo.lock` before the PR merges (no
  floating version requirements).

### Secrets and credentials

- Never commit tokens, keys, or credentials. The `.gitignore` is not a
  safety net.
- If you see a potential secret in the codebase, flag it to the maintainer
  immediately rather than committing over it.

---

## Code standards

- `cargo fmt --all` must be clean.
- `cargo clippy --all-targets -- -D warnings` must be clean.
- No bare `println!` / `eprintln!` debug output in library code — use the
  diagnostic infrastructure.
- Diagnostics carry `line:col:` source positions.
- New built-in functions: add a doc-comment and a test.
- New language features: add an `.expected.txt` golden sidecar in
  `resilient/examples/`.

---

## CI gates (all must pass)

| Check | Command |
|---|---|
| Build | `cargo build --locked` |
| Tests | `cargo test --locked` |
| Clippy | `cargo clippy --locked -- -D warnings` |
| Format | `cargo fmt --check` |
| Z3 | `cargo test --features z3` |
| Embedded cross | `cargo build --target thumbv7em-none-eabihf` etc. |
| Size gate | `.text` ≤ 64 KiB for Cortex-M4F demo |
| Perf gate | `cargo bench` regression check |
| Fuzz | short fuzz run on changed harnesses |

Do not open a PR for review until all CI jobs are green.

---

## What not to do

- Do not create planning documents or analysis files — work from conversation
  context and ticket bodies.
- Do not add comments that explain what code does — use well-named
  identifiers. Only add a comment when the *why* is non-obvious.
- Do not add error handling for impossible cases — trust internal invariants.
- Do not introduce backwards-compatibility shims for removed code.
- Do not half-implement a feature and leave a `TODO` — either finish it or
  scope it to a follow-up ticket.
