---
id: RES-072
title: Phase 5 Cranelift backend skeleton
state: DONE
priority: P2
goalpost: G15
created: 2026-04-17
owner: executor
---

## Summary
G15 / Phase 5: introduce a Cranelift JIT backend so Resilient programs
can run as native code instead of via the tree-walking interpreter. This
ticket is the SKELETON — wire `cranelift` and `cranelift-jit` as
dependencies and lower the smallest meaningful subset (integer literals,
+, -, *, function calls returning int, top-level `main`) to native code
behind a `--jit` flag. Float, strings, arrays, structs, contracts, and
live blocks are out of scope here and follow in dedicated tickets.

## Acceptance criteria
**Phase A scope (this ticket)**: foundation only — deps + feature
flag + driver flag + stub `jit_backend::run`. Real lowering of AST
to native code splits into RES-096+ follow-ups so each piece is
reviewable. Mirrors how RES-074 LSP landed in stages.

- New `jit` feature in `Cargo.toml`. Adds `cranelift = "0.108"` and
  `cranelift-jit = "0.108"` (or matching latest) as **optional**
  deps. Default builds untouched.
- New module `resilient/src/jit_backend.rs` (gated on `jit` feature)
  exposing a single `pub fn run(program: &Node) -> Result<i64, JitError>`
  that, for now, returns `Err(JitError::Unsupported("jit not implemented yet — RES-096+ adds AST lowering"))`.
- New CLI flag `--jit` on the driver: under `--features jit`, calls
  `jit_backend::run` and surfaces the error cleanly. Without the
  feature, prints the same helpful "rebuild with --features jit"
  message that `--lsp` shows (RES-074 pattern).
- New unit test in `jit_backend.rs` `mod tests`: confirms `run`
  returns `Unsupported`. Sanity check that the deps link.
- Build-only smoke test: `cargo build --features jit` succeeds.
  No runtime assertion yet — that's RES-096.
- `cargo build`, `cargo test`, `cargo clippy -- -D warnings` pass
  on default features and `--features jit`. (Z3 + LSP feature
  combinations remain green too.)
- Commit message: `RES-072: Cranelift JIT scaffolding (Phase A — deps + flag + stub)`.

**Out of scope (split into follow-ups):**
- AST lowering to Cranelift IR — RES-096 (smallest subset:
  IntegerLiteral + Add).
- Control flow + function calls — RES-097 (depends on RES-096).
- Top-level `Program` with a `main() -> int` runner that actually
  executes JIT'd code — RES-098.
- The fib bench under `--jit` — RES-099, depends on RES-097.

## Notes
- Mirrors RES-074 LSP scaffolding: opt-in feature flag + driver
  hook + stub backend, with real implementation in follow-ups.
  The pattern works because the dep tree is heavy and we don't
  want default builds carrying it.
- Cranelift 0.108's `cranelift-frontend` builder is the standard
  entry point. The first follow-up (RES-096) will set up the
  ISA + JIT module + a `main()` function that just returns 42.
- `JitError` variants suggested: `Unsupported(&'static str)` for
  this ticket; future tickets add `IsaInit(String)` for cranelift
  setup failures and `LinkError(String)` for module finalization.

## Log
- 2026-04-17 created by manager
- 2026-04-17 acceptance criteria filled in by manager
- 2026-04-17 manager rescope: Phase A only (deps + flag + stub).
  Real lowering split into RES-096+ follow-ups.
- 2026-04-17 executor landed Phase A:
  - Cargo.toml: new `jit` feature with optional cranelift +
    cranelift-jit deps (~150 transitive crates, ~14s cold build).
    Default builds untouched.
  - New `resilient/src/jit_backend.rs` (gated on `jit` feature):
    - `pub fn run(_program: &Node) -> Result<i64, JitError>` —
      stub returning `Unsupported("jit not implemented yet —
      RES-096+ adds AST lowering")`.
    - `JitError::Unsupported(&'static str)` enum + Display impl.
    - Imports `cranelift::prelude::*` and `cranelift_jit::*` at
      module level (with `#[allow(unused_imports)]`) so the build
      verifies the deps link cleanly. Real use lands with RES-096.
  - main.rs: new `--jit` driver flag. Under `--features jit` it
    dispatches to `jit_backend::run` and surfaces errors as
    `<filename>: <error>` (RES-095 shape). Without the feature it
    prints "rebuild with --features jit" and exits 1 (mirrors
    RES-074 LSP pattern).
  - 2 unit tests in `jit_backend::tests`:
    - `run_returns_unsupported_until_res_096`: confirms the stub
      returns Unsupported with a message pointing at RES-096.
    - `jit_error_display_is_descriptive`: Display formatting check.
- 2026-04-17 manual verification:
  - `cargo build` (default) → `--jit prog.rs` prints
    `--jit requires the jit feature. Rebuild with: cargo build
    --features jit`, exits 1.
  - `cargo run --features jit -- --jit prog.rs` prints
    `Error: prog.rs: jit: unsupported: jit not implemented yet —
    RES-096+ adds AST lowering`, exits 1.
- 2026-04-17 verification across four feature configs:
  - default: 217 unit + 1 golden + 12 smoke = 230 tests
  - `--features z3`: 225 + 1 + 13 = 239 tests
  - `--features lsp`: 221 + 1 + 12 + 3 lsp_smoke = 237 tests
  - `--features jit`: 219 + 1 + 12 = 232 tests
  All four `cargo clippy -- -D warnings` clean.
- 2026-04-17 follow-up tickets queued:
  - RES-096 — lower IntegerLiteral + Add to native, prove a real
    JIT-compiled function returns the right value.
  - RES-097 — add control flow + function calls to the JIT.
  - RES-098 — JIT'd `main() -> int` runner end-to-end.
  - RES-099 — fib(25) under `--jit` for the perf comparison.
