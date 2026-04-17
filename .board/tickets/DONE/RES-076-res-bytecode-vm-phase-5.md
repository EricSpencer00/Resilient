---
id: RES-076
title: Bytecode VM as the next perf win (Phase 5)
state: DONE
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
**Scope (this ticket — FOUNDATION ONLY):**
- New module `resilient/src/bytecode.rs` defining:
  - `enum Op { Const(u16), Add, Sub, Mul, Div, Mod, Neg, LoadLocal(u16), StoreLocal(u16), Return }`
  - `struct Chunk { code: Vec<Op>, constants: Vec<Value>, line_info: Vec<u32> }`
  - `enum CompileError { Unsupported(&'static str), TooManyConstants, TooManyLocals }`
  - `enum VmError { TypeMismatch(&'static str), DivideByZero, EmptyStack }`
- New module `resilient/src/compiler.rs` with `pub fn compile(program: &Node) -> Result<Chunk, CompileError>`. Supports the subset:
  - `IntegerLiteral`
  - `PrefixExpression "-"` and `InfixExpression + - * / %` over ints
  - `LetStatement` (no type-annot enforcement; just bind)
  - `Identifier` lookup against the local table
  - `ReturnStatement` (with value, or bare → returns Void)
  - `ExpressionStatement` wrapping the above
  - Top-level `Program` containing the above (bare statements at top level — no fn body needed since this is the foundation)
- New module `resilient/src/vm.rs` with `pub fn run(chunk: &Chunk) -> Result<Value, VmError>`. Stack-based interpreter with `Vec<Value>` operand stack and `Vec<Value>` locals slab.
- New CLI flag `--vm` runs the program through bytecode instead of the tree walker.
- Smoke test in `resilient/tests/` that runs `let x = 2 + 3 * 4; return x;` through the VM via the `--vm` flag and asserts the binary exits 0 with `14` somewhere in stdout.
- Unit tests in `bytecode.rs` / `vm.rs` covering: Const + Return, Add, Mul precedence (compile `2 + 3 * 4` and run it → 14), LetStatement + LoadLocal, divide-by-zero error, type mismatch (Add on a String constant).
- Anything outside the supported subset returns `CompileError::Unsupported(<&'static str describing what>)`. No panics.
- `cargo build`, `cargo test`, `cargo clippy -- -D warnings` all pass on default features and `--features z3`.
- Commit message: `RES-076: bytecode VM foundation (G15 partial)`.

**Out of scope (split into follow-ups):**
- Function calls + recursion → RES-081 (mint after this ships).
- The fib(30) ≥ 3× speedup benchmark → RES-082 (depends on RES-081).
- Control flow (if/while/for) → RES-083.
- Strings, arrays, structs, contracts, live blocks → individual follow-ups.

## Notes
- Stack-based is fine for the first cut; register-based is a follow-up.
- Use `u16` indices throughout — keeps Op size small and the cap (65536
  constants/locals/jumps) is more than enough for now.
- Crucially: this lives ALONGSIDE the tree walker, behind the flag. Do
  NOT delete the interpreter. We want both as oracles when fuzzing.
- For the smoke test, use the same `Command::new(bin())` pattern as
  the existing tests in `tests/examples_smoke.rs`. Write a temp file
  rather than committing a new example until the VM is feature-rich
  enough to be interesting.

## Log
- 2026-04-17 created by manager
- 2026-04-17 acceptance criteria filled in by manager
- 2026-04-17 manager pass: scope split into FOUNDATION (this ticket)
  + RES-081/082/083 follow-ups so each piece is shippable in one
  iteration.
- 2026-04-17 executor landed FOUNDATION:
  - `bytecode.rs`: `enum Op { Const, Add, Sub, Mul, Div, Mod, Neg,
    LoadLocal, StoreLocal, Return }` (u16 indices, 4-byte Op),
    `Chunk { code, constants, line_info }` with `add_constant` dedup
    and `emit` helper, `CompileError`/`VmError` enums with `Display`.
  - `compiler.rs`: `pub fn compile(&Node) -> Result<Chunk, CompileError>`
    walks the FOUNDATION subset (IntegerLiteral, prefix `-`, infix
    arith, LetStatement, Identifier, ReturnStatement, ExpressionStatement,
    Program). Locals resolved at compile time to `u16` slab indices
    via per-program HashMap. Unknown identifier → clean error;
    out-of-subset → `Unsupported(&'static str)`.
  - `vm.rs`: `pub fn run(&Chunk) -> Result<Value, VmError>`. Stack-
    based; wrapping arithmetic on i64; divide/mod-by-zero is a clean
    error; type mismatch reports the offending Op name; locals slab
    grows lazily on first store.
  - Driver: new `--vm` flag; when set, the program is compiled to a
    `Chunk` and run by `vm::run` instead of going through the tree
    walker. Print path mirrors the interpreter (skip Void).
- 2026-04-17 tests:
  - `bytecode::tests`: 4 tests covering constant dedup, distinct-
    keep, `emit` append, and `CompileError` Display.
  - `compiler::tests`: 5 tests covering int literal, arith
    precedence, let → StoreLocal, unknown identifier, unsupported
    construct.
  - `vm::tests`: 7 tests covering Const+Return, Add, end-to-end
    `2 + 3 * 4` via the real compiler, let+load, divide-by-zero,
    type-mismatch on Add(Int, String) hand-rolled chunk, negation.
    Helper `assert_int` destructures since `Value` doesn't impl
    `PartialEq` (Function variant carries `Box<Node>`).
  - `tests/examples_smoke.rs`: 2 new smoke tests:
    `bytecode_vm_runs_arithmetic_and_let` writes a temp file with
    `let x = 2 + 3 * 4; return x;`, runs `--vm`, asserts stdout
    contains `14`. `bytecode_vm_rejects_unsupported_construct_cleanly`
    writes `if true { let x = 1; }`, runs `--vm`, asserts non-zero
    exit + stderr mentions `VM compile error` or `unsupported`.
- 2026-04-17 manual end-to-end: `cargo run -- --vm /tmp/r76.rs` on
  the smoke-test source prints `14` then `Program executed successfully`.
- 2026-04-17 verification: 184 unit + 1 golden + 9 smoke = 194 tests
  default. With `--features z3`: 192 + 1 + 10 = 203. Clippy clean
  both ways.
- 2026-04-17 follow-ups to mint when their turn comes:
  - **RES-081**: function calls + recursion (CallFrame, Op::Call,
    Op::Return-from-call). Required for any non-trivial program.
  - **RES-082**: fib(30) microbench, target ≥3× over tree walker.
    Depends on RES-081.
  - **RES-083**: control flow (Op::Jump, Op::JumpIfFalse, Op::Loop)
    for if/while/for.
