---
id: RES-072
title: Phase 5 Cranelift backend skeleton
state: OPEN
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
- `Cargo.toml` adds `cranelift = "0.108"` and `cranelift-jit = "0.108"`
  (or whatever the latest matched pair is at the time) under a new
  `jit` feature flag — default off so non-JIT users don't pay the build
  cost.
- New module `resilient/src/jit.rs` with a `Jit` struct exposing
  `compile_and_run(program: &Node) -> Result<i64, JitError>`.
- Supports lowering: `IntegerLiteral`, `PrefixExpression -`, `InfixExpression + - *`, `LetStatement`, `Identifier`, `ReturnStatement`, single-function `Program` with a `main() -> int`.
- New CLI flag `--jit` on the driver runs the program through the new
  backend instead of the interpreter.
- Smoke test in `resilient/tests/` that JIT-runs a `.res` file computing
  `let x = 2 + 3 * 4; return x;` and asserts the process exits with 14.
- Anything outside the supported subset returns
  `Err(JitError::UnsupportedNode(...))` — never panics.
- `cargo build --features jit`, `cargo build` (no feature), and
  `cargo test --features jit` all pass.
- Commit message: `RES-072: Cranelift JIT skeleton (G15 partial)`.

## Notes
- Cranelift's `cranelift-frontend` builder pattern is the standard entry
  point. Use `cranelift_module::Linkage::Local` for the generated `main`.
- Keep the lowering pure — given an AST it should be deterministic and
  emit no I/O.
- Write a follow-up ticket for each missing feature (control flow,
  function calls, contracts) before closing this one.

## Log
- 2026-04-17 created by manager
- 2026-04-17 acceptance criteria filled in by manager
