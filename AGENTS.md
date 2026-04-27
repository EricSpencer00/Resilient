# AGENTS.md — Resilient

Instructions for OpenAI Codex, GPT-based agents, and other autonomous
coding assistants operating in this repository.

---

## Project overview

Resilient is a compiled, statically-typed language for safety-critical
embedded systems. It is written in Rust and ships two primary crates:

- `resilient/` — compiler, CLI, REPL, JIT, LSP
- `resilient-runtime/` — `#![no_std]` embedded runtime (no heap, no std)

This is an **agent-native** repository. AI agents are first-class
contributors and are expected to operate with high autonomy on well-scoped
issues from [GitHub Issues](https://github.com/EricSpencer00/Resilient/issues).

---

## Quick-start

```bash
cargo build --manifest-path resilient/Cargo.toml
cargo test  --manifest-path resilient/Cargo.toml
cargo test  --manifest-path resilient-runtime/Cargo.toml
cargo clippy --all-targets -- -D warnings
cargo fmt --all
```

Full setup, feature flags, and cross-compile instructions are in
[CONTRIBUTING.md](CONTRIBUTING.md).

---

## Ticket workflow

1. Browse [open issues](https://github.com/EricSpencer00/Resilient/issues?q=is%3Aissue+is%3Aopen).
2. Comment to claim, then create a branch `res-NNN-short-title`.
3. Open a draft PR early with `Closes #N` in the body.
4. Use `agent-scripts/ready-or-bail.sh --pr N` to leave draft state.
   Do not call `gh pr ready` directly for agent PRs.
5. On merge, the issue closes automatically.

Commit format: `RES-NNN: short description` (≤72 chars).

---

## What agents may do autonomously

- Claim and work open tickets end-to-end.
- Add new source files, tests, and `.expected.txt` golden sidecars.
- Fix clippy warnings and formatting issues anywhere in the codebase.
- Expand documentation (README, docs/, SYNTAX.md).
- Open and update draft PRs on feature branches.
- Write PR handoff comments with `agent-scripts/agent-handoff.sh` so work
  can resume after model context loss.

## What requires human approval

- **Any modification to existing tests** — unit tests, `.expected.txt`
  golden files, fuzz harnesses, or benchmark baselines. See "Test policy."
- New or modified `unsafe` blocks.
- Breaking changes to stable language surface (read STABILITY.md).
- Dependency version bumps beyond patch level.
- Changes to `.github/workflows/` CI definitions.
- Any action that bypasses CI or commit hooks.
- Any direct draft-to-ready transition that bypasses
  `agent-scripts/ready-or-bail.sh`.

---

## Test policy — CRITICAL

**All PRs must pass every test. PRs that modify existing tests require
explicit maintainer approval before merge.**

Rules:
1. A failing test means the *implementation* is wrong — fix the code, not
   the test.
2. Do not delete or weaken tests (lowering an assertion = deleting a test).
3. When a test legitimately needs updating (intentional behaviour change),
   call it out in a **"Test changes"** section of the PR body with a
   one-line rationale per modified test.
4. New behaviour must be covered by new tests — do not rely solely on
   existing coverage.

---

## Security rules

### No panics

- `resilient-runtime/` (no_std): zero panics. Use `Result`/`Option`.
  `unwrap()`/`expect()` in library code is a bug.
- `resilient/` parser and lexer: all errors must produce a typed diagnostic
  with `line:col:` position. A panic is a bug.

### `unsafe`

- Do not introduce `unsafe` without a comment explaining the invariant that
  makes it sound.
- Flag every new/modified `unsafe` block explicitly in the PR description.

### `no_std` constraints

- `resilient-runtime/` default build: no `Vec`, `String`, `Box`, or any
  other `std` type outside `#[cfg(feature = "alloc")]` gates.
- Cross-compile for embedded targets must remain green.

### Dependencies

- Do not add crates without a stated rationale in the PR.
- Prefer in-tree implementations for small utilities.

---

## CI — all checks must pass before review

`cargo build --locked` · `cargo test --locked` · `cargo clippy -- -D warnings`
· `cargo fmt --check` · embedded cross-compile · size gate (≤ 64 KiB .text)
· perf gate · fuzz

Agent PRs must also pass the local guardrail path:

```bash
agent-scripts/ready-or-bail.sh --pr <number>
```

That script runs `verify-scope.sh`, syncs through `agents/integration`,
posts a durable handoff comment, and marks the PR ready only after the
local gate is green.

---

## Do not

- Commit secrets, tokens, or credentials.
- Add `println!`/`eprintln!` debug output in library code.
- Create planning documents or analysis files as repo artifacts.
- Add comments that describe *what* code does — only add comments when the
  *why* is non-obvious.
- Leave half-implemented features with `TODO` markers — scope them to a
  follow-up ticket instead.
- Force-push or amend commits that have been reviewed.
