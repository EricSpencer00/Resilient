---
id: RES-081
title: VM function calls and recursion
state: DONE
priority: P1
goalpost: G15
created: 2026-04-17
owner: executor
---

## Summary
RES-076 landed the bytecode VM foundation (int arithmetic + let
bindings + return) but rejects function declarations with
`CompileError::Unsupported("function decl (RES-081)")`. This ticket
lifts that restriction: compile top-level `fn` bodies as separate
chunks, and add a `Call` op so any bytecode program can invoke them.
Recursion is the big test — once this lands, fib-style programs can
run under `--vm`.

Both pieces — function compilation and call semantics — have to
land together because one is useless without the other.

## Acceptance criteria
- New `bytecode::Function { name: String, arity: u8, chunk: Chunk, local_count: u16 }` type. A top-level `Node::Function` compiles to one `Function` value; parameters occupy the first `arity` slots of the function's locals slab.
- `bytecode::Program { main: Chunk, functions: Vec<Function> }` as the new top-level compile output. `compile()` signature updates to `pub fn compile(program: &Node) -> Result<Program, CompileError>`; the driver adapts.
- New ops: `Op::Call(u16)` takes a function-table index; the VM pops `arity` values from the operand stack as args and pushes a new `CallFrame`. `Op::ReturnFromCall` pops the current frame, puts the top-of-stack value (the return value) onto the caller's stack.
- `vm::run` grows a `Vec<CallFrame>` where each frame holds `{ return_chunk: *const Chunk, return_pc: usize, locals_base: usize }`. The main dispatch loop switches on the current frame's chunk.
- Compiler support: function-name → function-index table built in a pre-pass (mirrors the interpreter's forward-reference hoist). `CallExpression` with a known function name → `Call(idx)`; unknown name → `CompileError::UnknownFunction(name)` (new variant).
- Unit tests in `bytecode.rs` / `vm.rs`:
  - Single no-arg `fn zero() { return 0; } zero();` → 0
  - Unary `fn sq(int n) { return n * n; } sq(5);` → 25
  - Two-arg `fn add(int a, int b) { return a + b; } add(3, 4);` → 7
  - Stack underflow on hand-rolled bad chunk returns `VmError::EmptyStack` cleanly (no panic).
  - Unknown-function call surfaces `CompileError::UnknownFunction`.
- Smoke test in `tests/examples_smoke.rs`: `--vm` on a temp file with `fn sq(int n) { return n * n; } println(sq(7));` — expects stdout `49`. (Note: `println` is a builtin that currently only exists in the interpreter path; either the VM learns `Op::BuiltinCall` in a thin addition here, OR the smoke test uses the program's own `return sq(7);` and just checks for `49` in stdout.)
- `cargo build`, `cargo test`, `cargo clippy -- -D warnings` all pass on default features and `--features z3`.
- Commit message: `RES-081: VM function calls + recursion (G15 partial)`.

## Notes
- **Dependency on RES-083**: recursion tests that need branching (fib, fact) require `if` — that's RES-083's work. Unit tests in this ticket should exercise calls WITHOUT needing control flow. RES-083 + RES-082 will then land fib as a microbench.
- `CallFrame` is stack-local in the VM loop (no Box needed). The callframe stack is a `Vec<CallFrame>`.
- Locals slab becomes per-frame: on Call, grow `locals` by the callee's `local_count`, record `locals_base`. On ReturnFromCall, shrink back to `locals_base`. LoadLocal / StoreLocal indices are frame-relative — the VM adds `locals_base` before indexing.
- Max 255 params per function (u8 arity). If a program exceeds that, return `CompileError::Unsupported("fn with >255 params")`. Real programs never hit this.
- Tree walker is the oracle — for any program the VM accepts, the result should match the interpreter. Cross-check that `sq(5)` through both paths returns the same Int.

## Log
- 2026-04-17 created by manager
- 2026-04-17 acceptance criteria filled in by manager (orchestrator pass)
- 2026-04-17 executor landed:
  - `bytecode::Function { name, arity: u8, chunk, local_count: u16 }`
    + `bytecode::Program { main, functions }`. Compiler's return type
    changed from `Chunk` to `Program`.
  - New ops `Op::Call(u16)` and `Op::ReturnFromCall`.
    `Call` pops `arity` args (leftmost popped last → stored into
    slots `0..arity` in source order), reserves `local_count` locals,
    pushes a `CallFrame`. `ReturnFromCall` pops a return value,
    unwinds the frame, pushes the value onto the caller's stack.
    Top-level `return` still emits `Op::Return` (halts); inside a
    fn body `return` emits `ReturnFromCall`.
  - New `CompileError::UnknownFunction(String)` and
    `CompileError::ArityMismatch { ... }`; new `VmError` variants
    `FunctionOutOfBounds`, `CallStackUnderflow`, `CallStackOverflow`
    with a 1024-frame safety cap to stop runaway recursion.
  - Compiler pre-pass: function-name → index table built before
    body compilation so forward references work. Each fn body
    compiled with its own locals map starting with params in slots
    `0..arity`.
  - VM uses a shared `locals: Vec<Value>` slab plus a per-frame
    `locals_base` offset so LoadLocal/StoreLocal indices remain
    frame-relative. `ReturnFromCall` truncates `locals` back to
    the caller's base — no leaks across frames.
- 2026-04-17 tests: **12 new unit tests** across compiler (5) and
  vm (7), including the oracle test
  `vm_and_tree_walker_agree_on_call_result` that cross-checks VM and
  interpreter on `sq(6) = 36`. Existing tests updated for the new
  `Program` return type. New smoke test `bytecode_vm_runs_fn_call`
  runs `fn sq(int n) { return n * n; } sq(7);` through `--vm` and
  asserts stdout contains `49`.
- 2026-04-17 manual verification: nested-call test
  `add(sq(3), sq(4)) = 25` runs correctly through `--vm`.
- 2026-04-17 build/test/clippy: 196 unit + 1 golden + 10 smoke =
  207 tests default; 204 + 1 + 11 = 216 with `--features z3`.
  Clippy clean both ways.
- Recursion with terminating branches is deferred to RES-083
  (control flow) + RES-082 (fib bench) per the ticket's explicit
  dependency note.
