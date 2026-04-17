---
id: RES-083
title: VM control flow if while for
state: OPEN
priority: P1
goalpost: G15
created: 2026-04-17
owner: executor
---

## Summary
RES-076 + RES-081 gave the VM arithmetic, let bindings, and function
calls — but there's still no way to branch or loop. This ticket adds
`Op::Jump(i16)` and `Op::JumpIfFalse(i16)` (signed offsets relative
to the PC after the jump, so positive jumps forward and negative
loops back), plus compiler paths for `if` / `while` / boolean infix
operators (`==`, `!=`, `<`, `<=`, `>`, `>=`, `&&`, `||`) and boolean
literals. Once this lands, terminating recursion works → fib is
computable → RES-082's 3× bench is writable.

`for .. in` is out of scope (requires iterator protocol / array
ops); that stays a follow-up.

## Acceptance criteria
- New ops:
  - `Op::Jump(i16)` — unconditional relative jump from the PC *after*
    the op to `PC + offset`.
  - `Op::JumpIfFalse(i16)` — pop the top of the operand stack; if
    falsy (`Bool(false)` or `Int(0)`), jump by the offset; otherwise
    fall through. Non-bool/non-int top of stack is a
    `VmError::TypeMismatch("JumpIfFalse")`.
  - `Op::Eq`, `Op::Neq`, `Op::Lt`, `Op::Le`, `Op::Gt`, `Op::Ge`: pop
    two ints, push `Value::Bool`. Type mismatch reports the op name.
  - `Op::Not`: pop a bool, push its negation.
- Compiler:
  - `Node::BooleanLiteral(b)` → `Const` with `Value::Bool(b)`.
  - `Node::PrefixExpression "!"` → `Not`.
  - `Node::InfixExpression` — ops listed above lowered to the right
    bytecode. `&&` and `||` emit short-circuit using `JumpIfFalse`
    / a new `JumpIfTrue` helper (or simulate with Not + JumpIfFalse).
  - `Node::IfStatement { condition, consequence, alternative }` —
    compile cond, `JumpIfFalse else_or_end`, compile consequence,
    `Jump end`, back-patch the skip targets once positions are known.
  - `Node::WhileStatement { condition, body }` — compile cond at
    `loop_start`, `JumpIfFalse end`, compile body, `Jump loop_start`.
  - `for .. in` is NOT in scope — returns `Unsupported("for-in")`.
- VM: new ops land in the dispatch loop. Jump offsets are `i16`,
  applied to `pc as isize + offset as isize`; out-of-range target
  is a clean `VmError::JumpOutOfBounds`.
- Unit tests in `bytecode.rs` / `compiler.rs` / `vm.rs`:
  - `if true { 1; } else { 2; }` → 1 via VM
  - `if false { 1; } else { 2; }` → 2 via VM
  - `if` without an `else` and a falsy condition returns Void
  - `while` counting loop: `let i = 0; let sum = 0; while i < 5 { sum = sum + i; i = i + 1; } sum;` → 10 (depends on RES-078/079 not being needed — `Assignment` is already a supported node; if not supported in VM yet, add a simple StoreLocal-on-known-ident path here too or use `let` shadowing)
  - Recursive fib: `fn fib(int n) { if n <= 1 { return n; } return fib(n - 1) + fib(n - 2); } fib(10);` → 55 via VM
  - Cross-check: same fib program through the tree walker returns 55
  - Equality ops return Bool; comparison with a float operand is a type mismatch error (we only support int comparison in this first cut)
- Integration smoke test in `tests/examples_smoke.rs`: `--vm` on a temp file with the fib(10) program, expects `55` in stdout.
- `cargo build`, `cargo test`, `cargo clippy -- -D warnings` all pass on default features and `--features z3`.
- Commit message: `RES-083: VM control flow (if/while) + boolean ops (G15 partial)`.

## Notes
- **Relative offsets** not absolute PCs: keeps the chunk relocatable
  and matches common VM practice (crafted-interpreters-style
  patching). After emitting the jump, the compiler notes the patch
  site, compiles the target, computes `target_pc - (patch_pc + 1)`
  and writes it back into the jump's operand.
- `i16` gives ±32768 offset — plenty for one function body. If a
  body needs more, return `CompileError::JumpOutOfRange` and split.
- **Short-circuit** for `&&`: `a && b` compiles as: compile a; dup;
  JumpIfFalse end; Pop; compile b; end:. We don't have `Dup` or
  `Pop` yet — simplest alternative is desugar to `if a { b } else
  { false }` at compile time, which requires only JumpIfFalse +
  Jump. Do that.
- For `||`: `a || b` → `if a { true } else { b }`. Same desugar.
- `Node::Assignment` (for the `while` loop test) is currently
  unsupported in the VM — add a minimal path here: compile RHS,
  look up existing local by name, StoreLocal. If the name isn't in
  `locals`, `UnknownIdentifier`.
- Keep the tree walker's behavior as the oracle for `fib(10)` — the
  crate has an existing `Interpreter` that can eval the same AST.

## Log
- 2026-04-17 created by manager
- 2026-04-17 acceptance criteria filled in by manager (orchestrator pass)
