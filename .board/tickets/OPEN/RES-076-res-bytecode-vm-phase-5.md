---
id: RES-076
title: Bytecode VM as the next perf win (Phase 5)
state: OPEN
priority: P2
goalpost: G15
created: 2026-04-17
owner: executor
---

## Summary
The tree-walking interpreter is the perf floor. Cranelift (RES-072) is
the perf ceiling but a heavy dependency to take on every program. A
register-based bytecode VM sits in between: ~5-10× speedup over tree
walking with a small, embeddable runtime — perfect for the no_std target
(RES-075). This ticket is the FIRST CUT: a `Chunk` of bytecode, a
compiler from `Node` to `Chunk`, and a stack VM that executes it for
the same subset RES-072 supports (int arithmetic + let bindings + main).

## Acceptance criteria
- New module `resilient/src/bytecode.rs` defining:
  - `enum Op { Const(u16), Add, Sub, Mul, Div, Mod, Neg, LoadLocal(u16), StoreLocal(u16), Return }`
  - `struct Chunk { code: Vec<Op>, constants: Vec<Value>, line_info: Vec<u32> }`
- New module `resilient/src/compiler.rs` with `pub fn compile(program: &Node) -> Result<Chunk, CompileError>`.
- New module `resilient/src/vm.rs` with `pub fn run(chunk: &Chunk) -> Result<Value, VmError>`.
- New CLI flag `--vm` runs the program through bytecode instead of the tree walker.
- Microbench in `benchmarks/`: fib(30) under `--vm` is at least 3× faster than the default tree walker on the developer's machine. Add a paragraph to `benchmarks/RESULTS.md` with the measured numbers.
- Smoke test in `resilient/tests/` that compiles + runs `let x = 2 + 3 * 4; return x;` through the VM and asserts 14.
- Anything outside the supported subset returns a clean `CompileError::Unsupported(node_kind)`. No panics.
- `cargo build`, `cargo test`, `cargo clippy -- -D warnings` all pass.
- Commit message: `RES-076: bytecode VM (G15 partial, paves no_std path)`.

## Notes
- Stack-based is fine for a first cut; register-based is a follow-up.
- Use `u16` indices throughout — keeps Op size small and the cap (65536
  constants/locals/jumps) is more than enough for now.
- Crucially: this lives ALONGSIDE the tree walker, behind the flag. Do
  NOT delete the interpreter. We want both as oracles when fuzzing.

## Log
- 2026-04-17 created by manager
- 2026-04-17 acceptance criteria filled in by manager
