# GitHub Copilot Instructions — Resilient

## What is this repo?

Resilient is a compiled, statically-typed language for safety-critical
embedded systems, written in Rust. Two primary crates:

- `resilient/` — compiler, CLI, REPL, JIT, LSP server
- `resilient-runtime/` — `#![no_std]` embedded runtime (no heap in default build)

Agent and AI contributions are welcome. Read CONTRIBUTING.md for the full
workflow.

---

## Code standards Copilot must follow

### No panics

- In `resilient-runtime/` (library code): `unwrap()` and `expect()` are
  bugs. Use `Result` or `Option` and propagate.
- In `resilient/` parser and lexer: every error must return a typed `Error`
  with a `line:col:` diagnostic. Panics are bugs.

### `no_std` in `resilient-runtime/`

- No `std` types (`Vec`, `String`, `Box`) in the default feature set.
- Gate heap usage behind `#[cfg(feature = "alloc")]`.

### `unsafe`

- Only introduce `unsafe` when strictly necessary.
- Always include a comment explaining the invariant that makes the block sound.

### Style

- `cargo fmt --all` clean.
- `cargo clippy --all-targets -- -D warnings` clean (zero warnings).
- No bare `println!`/`eprintln!` in library code — use the diagnostic
  infrastructure.
- Diagnostics must include `line:col:` source positions.

---

## Test rules

**Never delete, weaken, or skip a test to make a PR pass. Fix the
implementation instead.**

- New language features need a `.expected.txt` golden sidecar in
  `resilient/examples/`.
- New built-in functions need a unit test.
- Modifications to existing tests require maintainer approval.

PRs that fail any CI test will not be merged.

---

## Pull request requirements

- Commits: `RES-NNN: short description` (≤72 chars).
- All CI checks must be green: build, test, clippy, fmt, embedded
  cross-compile, size gate, perf gate.
- Link the GitHub issue (`Closes #N`) in the PR body.
- Move the `.board/` ticket from `IN_PROGRESS/` to `DONE/` in the same PR.

See [CONTRIBUTING.md](../CONTRIBUTING.md) for the full checklist.
