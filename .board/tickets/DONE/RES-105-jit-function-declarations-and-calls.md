---
id: RES-105
title: JIT lowers function declarations and calls (RES-072 Phase H)
state: OPEN
priority: P2
goalpost: G15
created: 2026-04-17
owner: executor
---

## Summary
Phases B–G grew the JIT from "single return expression" to "let
bindings + if/else + comparisons + arithmetic at the top
level." Phase H is the next plateau: user-defined functions
with parameters, called from the program's entrypoint or from
each other.

After this ticket the JIT can compile programs like:

```
fn square(int x) { return x * x; }
return square(7);
```

…and (with RES-104 already in place) eventually `fib`. RES-106
takes the next step and benchmarks JIT'd fib against the
bytecode VM (currently 32 ms on fib(25)).

## Acceptance criteria
- Two-pass compilation of `Node::Program(stmts)`:
  - **Pass 1**: walk top-level statements, find every
    `Node::Function { name, parameters, body, .. }`. For each:
    build a Cranelift signature with N i64 params + 1 i64
    return, declare the function in the JITModule, and stash
    the FuncId in a `HashMap<String, FuncId>` keyed by name.
  - **Pass 2**: compile each declared function's body with a
    fresh LowerCtx whose locals map is pre-populated with the
    parameter Variables. Compile the program's entrypoint
    (the top-level non-Function statements) as `main`, exactly
    as today.
- New helper `compile_function`:
  - Takes the FuncId, parameter list, body, and the
    `HashMap<String, FuncId>` for cross-function calls.
  - Builds a fresh function context, declares one Variable per
    parameter (matching the Spanned param list), `def_var`s
    each from the entry block's params, then calls
    `compile_statements` on the body.
  - Same EmptyProgram error if the body never returns.
- `lower_expr` adds an arm for `Node::CallExpression`:
  - Resolves the callee. Today only direct calls are
    supported: `function` must be a `Node::Identifier { name }`.
    Anything else (closures, method calls) returns
    Unsupported with the descriptor "JIT only supports
    direct calls (Identifier callee)".
  - Looks up the FuncId in the context's function map. Missing
    → Unsupported("call to unknown function: NAME") with the
    name in the message so users can debug typos.
  - Lowers each argument into a `Vec<Value>`.
  - Validates arg count matches the declared parameter count.
    Mismatch → Unsupported with the descriptor
    "arity mismatch: NAME expected N, got M".
  - Declares the function as a local function ref via
    `module.declare_func_in_func(func_id, &mut bcx.func)`,
    then `bcx.ins().call(local_func_ref, &args)` and
    `bcx.inst_results(call)[0]` to get the return value.
- LowerCtx grows to carry the function map (or a separate
  `FunctionCtx` is threaded alongside — pick whichever keeps
  the signature noise tolerable). The module reference also
  needs threading because `declare_func_in_func` is a
  `&mut Module` method.
- New unit tests in `jit_backend::tests`:
  - `jit_calls_zero_arg_function`:
    `fn answer() { return 42; } return answer();` → 42
  - `jit_calls_one_arg_function`:
    `fn square(int x) { return x * x; } return square(7);` → 49
  - `jit_calls_two_arg_function`:
    `fn add(int a, int b) { return a + b; } return add(3, 4);` → 7
  - `jit_calls_with_local_then_call`:
    `fn square(int x) { return x * x; } let y = 5; return square(y) + 1;` → 26
  - `jit_recursive_call_factorial`:
    classic factorial → `factorial(5)` → 120. Proves the
    function map supports calls to the function being
    compiled (forward reference within Pass 2).
  - `jit_call_unknown_function_unsupported`:
    `return undefined_fn();` → Unsupported with
    "unknown function: undefined_fn"
  - `jit_call_arity_mismatch_unsupported`:
    `fn f(int x) { return x; } return f(1, 2);` → Unsupported
    with "arity mismatch"
- Smoke test in `tests/examples_smoke.rs` (gated `--features jit`):
  `fn double(int x) { return x + x; } return double(21);` →
  driver prints 42 and exits 0.
- All four feature configs pass `cargo test` and
  `cargo clippy --all-targets -- -D warnings`.
- Commit message: `RES-105: JIT lowers function decls + calls (RES-072 Phase H)`.

## Notes
- Cranelift function declaration:
  ```rust
  let mut sig = module.make_signature();
  for _ in &parameters { sig.params.push(AbiParam::new(types::I64)); }
  sig.returns.push(AbiParam::new(types::I64));
  let func_id = module.declare_function(&name, Linkage::Local, &sig)?;
  ```
- Inside compile_function, after `bcx.append_block_params_for_function_params(entry)`,
  iterate `bcx.block_params(entry)` to get the Cranelift Values
  for each parameter, then declare a Variable per param and
  def_var with that Value.
- For recursive calls: declare ALL functions first (Pass 1),
  THEN compile bodies (Pass 2). At Pass 2 time, every function
  including the one being compiled has a FuncId in the map, so
  recursion just works.
- For mutual recursion: same trick — Pass 1 sees both
  declarations before Pass 2 compiles either body.
- Don't try to support `fn`s nested inside `fn`s (closures with
  upvalues) yet. Top-level fns only. The interpreter and VM
  support nested fns; the JIT can catch up later.
- `parameters: Vec<(String, String)>` is `(type, name)` per
  the AST definition — read the type for future contract
  integration but ignore it for lowering (everything is i64
  in Phase H).
- Multi-function programs with NO top-level non-function
  statements (e.g. just `fn f() { ... } fn g() { ... }`) —
  return EmptyProgram, same as a program with no return at
  all. The user must write `return f();` (or similar) at the
  top level to give `main` something to call.

## Log
- 2026-04-17 created by manager (Phase H scope, unblocks RES-106)
- 2026-04-17 executor: two-pass compilation in run() — Pass 1
  walks top-level statements building (HashMap<String, FuncId>,
  HashMap<String, usize>) of name → (FuncId, arity); Pass 2
  compiles each function body via the new compile_function
  helper, then compiles the program's non-function statements
  as `__resilient_main__`. compile_statements skips Function
  nodes (Pass 1 already declared them; Pass 2 compiles their
  bodies separately) so the entrypoint walker doesn't double-
  emit. lower_expr added a CallExpression arm: only
  Identifier callees supported (closures/methods are future
  tickets), missing fn → Unsupported("call to unknown
  function"), arity mismatch → Unsupported("call arity
  mismatch"), arg lowering then declare_func_in_func + ins().call.
  &mut JITModule threaded through compile_statements,
  compile_node_list, lower_if_statement, lower_block_or_stmt,
  and lower_expr — needed at the call site for declare_func_in_func.
  LowerCtx grew functions + function_arities owned maps
  (cloned from the program-wide map per compile, since the
  module is mutably borrowed during body lowering and we
  can't hold both).
  Nine new unit tests cover: 0/1/2-arg calls, call composing
  with locals, recursive factorial(5)=120, recursive
  fib(25)=75025 (the bench workload!), unknown-fn error,
  arity-mismatch error, mutual recursion (is_even/is_odd
  via two-pass declaration). Smoke test
  bytecode_jit_runs_function_call: `fn double(int x) { return
  x + x; } return double(21);` → driver prints 42, exits 0.
  Matrix: default 217, z3 225, lsp 221, jit 264 (+47 jit unit
  tests + 7 jit smoke tests since jit feature first existed).
  Clippy clean across all four configs. RES-106 (bench JIT'd
  fib vs VM) is now unblocked.
