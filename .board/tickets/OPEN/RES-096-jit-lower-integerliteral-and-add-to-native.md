---
id: RES-096
title: JIT — lower IntegerLiteral + Add to native (RES-072 Phase B)
state: OPEN
priority: P2
goalpost: G15
created: 2026-04-17
owner: executor
---

## Summary
RES-072 Phase A landed the Cranelift dep tree, the `--jit` flag,
and the stub `jit_backend::run`. This ticket is Phase B: actually
lower the smallest meaningful subset of the AST to native code and
**execute** it. Goal: a Resilient program containing only
`return 2 + 3;` returns 5 via the JIT path.

The Phase A stub returns `JitError::Unsupported`; this ticket
replaces that with a real `JITModule` setup, an `i64 () -> i64`
function whose body is the lowered AST, and a function-pointer
call to invoke it.

## Acceptance criteria
- `jit_backend::run(&Node)` succeeds for the subset:
  - `Node::IntegerLiteral { value, .. }` lowers to a constant.
  - `Node::InfixExpression` with operator `+` lowers to
    `InstBuilder::iadd`.
  - `Node::ReturnStatement { value: Some(expr), .. }` at top
    level lowers `expr` and returns the result.
  - `Node::Program` with the expected shape (single
    `ReturnStatement` at the top level) wraps as the JIT'd `main`.
- Anything outside that subset returns
  `JitError::Unsupported(<descriptor>)` — never panics.
- New `JitError` variants:
  - `IsaInit(String)` — cranelift_native isa builder failure.
  - `LinkError(String)` — JITModule::finalize_definitions failure.
  - `EmptyProgram` — no `return` at top level.
- New unit tests in `jit_backend.rs` `mod tests`:
  - `jit_returns_constant_42`: program is `return 42;`, asserts
    `run` returns `Ok(42)`.
  - `jit_adds_two_constants`: program is `return 2 + 3;`, asserts
    `run` returns `Ok(5)`.
  - `jit_rejects_let_for_now`: program with a `let` returns
    `Unsupported` cleanly.
- Smoke test in `tests/examples_smoke.rs` (gated `--features jit`):
  writes a temp file with `return 7 + 14;`, runs `--jit`, asserts
  stdout contains `21` and exits 0.
- `cargo build --features jit`, `cargo test --features jit`,
  `cargo clippy --features jit -- -D warnings` all pass.
- All other feature configs (default, `--features z3`,
  `--features lsp`) continue to pass — the JIT module is gated.
- Commit message: `RES-096: JIT lowers Const + Add — first real native execution`.

## Notes
- Cranelift 0.108 setup pattern:
  ```rust
  let mut flag_builder = settings::builder();
  flag_builder.set("use_colocated_libcalls", "false")?;
  flag_builder.set("is_pic", "false")?;
  let isa_builder = cranelift_native::builder()
      .map_err(|e| JitError::IsaInit(e.to_string()))?;
  let isa = isa_builder.finish(settings::Flags::new(flag_builder))?;
  let builder = JITBuilder::with_isa(isa, cranelift_module::default_libcall_names());
  let mut module = JITModule::new(builder);
  ```
- After `module.finalize_definitions()`, `module.get_finalized_function(func_id)`
  returns a `*const u8`. Cast to `unsafe extern "C" fn() -> i64`
  and call. Mark the call site `unsafe`.
- Keep the lowering pure — given an AST it should be deterministic.
- `cranelift-native` may need adding as a direct dep (Phase A only
  pulled in `cranelift` + `cranelift-jit`).

## Log
- 2026-04-17 created by manager
- 2026-04-17 acceptance criteria filled in by manager (orchestrator pass)
