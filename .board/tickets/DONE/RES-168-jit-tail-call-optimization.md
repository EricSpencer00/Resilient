---
id: RES-168
title: JIT: tail-call optimization for direct self-recursion
state: DONE
priority: P3
goalpost: G15
created: 2026-04-17
owner: executor
---

## Summary
Tail calls matter for functional-style programs — without TCO,
deep recursion overflows the host stack. Start small: only direct
self-recursion in tail position, same signature. Mutual TCO and
cross-function tail calls are harder and can wait.

## Acceptance criteria
- Lowering detects `return fn_name(args)` where `fn_name` is the
  enclosing function and arg count matches.
- Lowers to: evaluate args into the parameter variables, then
  `bcx.ins().jump(entry_block, &[])`. No `return` instruction is
  emitted on this path.
- SSA handling: the parameter Variables are re-`def_var`'d; phi
  construction at `entry_block` is handled by Cranelift's
  FunctionBuilder since entry now has ≥ 2 preds (original + tail
  back-edge).
- Unit tests: tail-recursive `count(n)` reaches `n = 1_000_000`
  without stack overflow; non-tail-position call to self is NOT
  optimized (still uses regular call, still stacks up).
- `benchmarks/jit/tail_rec.rs` added; numbers in RESULTS.md.
- Commit message: `RES-168: JIT TCO for direct self-recursion`.

## Notes
- Don't conflate with `tailcall` instruction (Cranelift 0.101+
  proposal) — we're doing the equivalent with a jump to the entry
  block, which is portable.
- Skip over tail calls wrapped in a `live` block — the retry
  semantics (RES-036) mean the call is NOT in tail position; the
  block boundary requires the activation.

## Log
- 2026-04-17 created by manager
- 2026-04-17 claimed by executor
- 2026-04-17 done by executor

## Resolution
- `resilient/src/jit_backend.rs`:
  - `LowerCtx` gains three TCO fields: `current_fn: Option<String>`
    (name of the enclosing function — `None` at top-level
    `main`), `tco_target: Option<Block>` (the body block jumped
    to on tail calls), and `param_vars: Vec<Variable>` (parameter
    Variables in declaration order for back-edge re-assignment).
  - `compile_function` now threads the function's name as a new
    `fn_name: &str` argument, and splits the function's
    Cranelift IR into **entry** + **body** blocks:
    - `entry` — carries the ABI block params, `def_var`s each
      into a parameter Variable, sealed immediately.
    - `body` — the actual function body lives here; left
      unsealed during lowering so tail-call back-edges can add
      predecessors. Sealed after lowering completes so
      Cranelift's SSA construction can insert phi nodes
      automatically at the back-edge merge points.
  - New `try_lower_tail_call(expr, bcx, ctx, module)` helper.
    Returns `Ok(true)` when the expression qualifies as a
    direct self-recursive tail call (name matches `current_fn`,
    arity matches `param_vars.len()`) and has been lowered as
    a back-edge jump. Argument expressions are lowered FIRST
    (so they reference the current parameter values) before
    any `def_var` reassignment. Returns `Ok(false)` for any
    non-qualifying shape, letting the caller fall back to the
    normal `return_` path.
  - Both `ReturnStatement` handlers (in `compile_node_list`
    for top-level / flat-fn-body walks and in
    `lower_block_or_stmt` for nested blocks inside if/while)
    now call `try_lower_tail_call` first. Top-level calls
    never qualify (top-level has `current_fn = None`), so
    top-level `main` behaviour is unchanged.
  - Cranelift's SSA construction handles the phi inference at
    `body`'s entry — no manual phi emission needed — because
    `def_var` calls at divergent predecessors are the exact
    trigger for auto-phi.
- `benchmarks/jit/tail_rec.rs` (new): accumulator-style
  `sum(n, 0)` at n = 1_000_000 exercises the full TCO path.
- `benchmarks/jit/RESULTS.md` (new): JIT TCO timings at 1M, 10M,
  100M (linear scaling: 4.4 ms → 7.2 ms → 74 ms); tree-walker
  stack-overflow reference at 5k as a foil showing why TCO
  matters here.
- Deviations from the ticket: none on the std / JIT path. The
  ticket's Notes about "skip over tail calls wrapped in a
  `live` block" is not exercised because `Node::LiveBlock`
  isn't JIT-lowered at all today — it surfaces as
  `JitError::Unsupported` earlier in the pipeline, so no TCO
  shortcut can mis-fire on it.
- Unit tests (5 new, all behind `--features jit`):
  - `tco_one_million_deep_recursion_does_not_stack_overflow` —
    ticket AC. `count(1_000_000)` completes cleanly; without
    TCO the host stack overflows around n ≈ 2–3k.
  - `tco_accumulator_style_sums_1_to_100k` — two-parameter
    TCO. Asserts exact value 5_000_050_000. Exercises the
    ordering requirement (args evaluated before any re-
    assignment) because the second arg `acc + n` depends on
    the OLD value of `n`.
  - `tco_only_fires_on_direct_self_recursion_not_wrapped_calls`
    — ticket AC. `return 1 + inc_count(n - 1)` still uses a
    regular call (not in tail position).
  - `tco_only_fires_on_matching_arity` — arity mismatch still
    surfaces the existing `arity` diagnostic; TCO doesn't
    mask it.
  - `tco_does_not_apply_to_cross_function_tail_calls` — the
    ticket is scoped to direct self-recursion; a cross-
    function tail call uses the normal call path and remains
    correct (though not stack-safe at arbitrary depth).
- Verification:
  - `cargo test --locked --features jit` — 503 passed (was
    498 before RES-168)
  - `cargo test --locked` — 445 passed (no regression)
  - `cargo test --locked --features logos-lexer` — 446 passed
  - `cargo clippy --locked --features logos-lexer,z3,jit
    --tests -- -D warnings` — clean
  - Manual bench (release build):
    `./resilient/target/release/resilient --jit
    benchmarks/jit/tail_rec.rs` → 500_000_500_000 in ~4 ms.
