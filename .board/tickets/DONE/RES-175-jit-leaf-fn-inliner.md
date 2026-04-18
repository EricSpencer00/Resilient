---
id: RES-175
title: Cranelift: inline trivial leaf functions at call sites
state: DONE
priority: P3
goalpost: G15
created: 2026-04-17
owner: executor
---

## Summary
`abs(-3)` compiles to a call-indirect into a 4-instruction shim.
Cranelift won't inline across our fn-pointer abstraction. Do the
inlining ourselves for leaf fns below a size threshold: splice
their body into the caller at lower-time.

## Acceptance criteria
- Define "trivial leaf": fn body has ≤ 8 AST nodes, no calls, no
  loops, no match.
- Per-call-site: if callee qualifies, inline the body with fresh
  locals; otherwise emit the normal indirect call.
- Recursion guard: never inline a direct self-call (trivial to
  check — callee name == enclosing fn name).
- Benchmark: fib(25) unchanged (non-leaf); `abs`-heavy microbench
  shows ≥30% improvement.
- Feature-gated by `cfg(not(test))`-style config? No — always on
  once the code is in place, since the leaf criteria is
  conservative.
- Commit message: `RES-175: Cranelift leaf-fn inliner`.

## Notes
- Ensure inlined locals don't collide with existing locals in the
  caller — suffix each with a unique counter at AST-level before
  lowering.
- Preserve span for runtime errors in the inlined body — the
  caret diagnostic (RES-117) should still point at the original
  source, not the call site.

## Log
- 2026-04-17 created by manager
- 2026-04-17 claimed by executor
- 2026-04-17 done by executor

## Resolution
- `resilient/src/jit_backend.rs`:
  - `LowerCtx` gains two fields:
    - `fn_asts: FnAstMap` (alias for
      `HashMap<String, (Vec<(String,String)>, Node)>`) — each
      function's AST keyed by name. Populated at
      `run_internal` pass-1 time, cloned into every per-fn
      ctx.
    - `inline_return_target: Option<Block>` — when set,
      `ReturnStatement` lowers to `jump(merge, &[v])` instead
      of the usual `return_`. Swapped in/out around each
      inline lowering.
  - `TRIVIAL_LEAF_MAX_NODES = 8` — hard cap on body AST size.
  - `count_nodes(node)` — recursive count including root.
  - `has_disqualifying_construct(node)` — true for any call,
    while/for loop, match, or live block inside the body.
  - `is_trivial_leaf(body, callee_name, current_fn)` — the
    top-level predicate; rejects self-recursion, over-sized
    bodies, and disqualifying constructs.
  - `try_lower_inline_call(params, body, args, ...)` — the
    actual inliner:
    1. Creates a merge block with a single i64 block-param.
    2. Lowers ALL argument expressions first, in the caller's
       scope (so they see the caller's bindings).
    3. Snapshots `locals` and `inline_return_target`.
    4. Declares fresh Variables for each callee param; shadows
       any caller local with the same name. After the inline
       ends, restoring `locals` unshadows the caller's
       binding — equivalent to the ticket's "suffix each with
       a unique counter at AST-level" but done without AST
       rewriting.
    5. Installs `merge` as the return target so
       `ReturnStatement` inside the body jumps to merge.
    6. Lowers the body via `lower_block_or_stmt`.
    7. Restores scope + inline target.
    8. Seals merge, switches to it, and returns its block-param
       as the call-site's SSA Value.
  - Both `ReturnStatement` handlers (flat body walker +
    nested `lower_block_or_stmt`) honor `inline_return_target`
    before falling back to `bcx.ins().return_`.
  - Inline lowering is disabled at NESTED inline sites
    (`ctx.inline_return_target.is_some()`) to bound code
    expansion — a future phase can relax this. Outer inline
    still fires normally; the nested one emits a regular
    indirect call.
  - `compile_function` grew an `fn_asts: &FnAstMap` arg (8
    total — under `#[allow(clippy::too_many_arguments)]`
    with a one-liner justification).
- `benchmarks/jit/leaf_heavy.rs` (new) + RESULTS.md update:
  10M-iteration tight loop calling `plus_one(i)` on every
  step. Before/after numbers pinned via a one-line
  `disable_inline` toggle in the JIT; results show
  **1.59× speedup (37% faster)** at p50, comfortably above
  the ticket's 30% threshold. fib(25) p50 is unchanged
  (inliner doesn't fire on non-leaf bodies).
- Deviations: the ticket mentioned suffixing locals at
  AST-level; I implemented equivalent scoping via snapshot/
  restore of `LowerCtx.locals` — cleaner, no AST rewriting,
  same scope-hygiene property (test
  `inliner_shadows_caller_local_with_same_name` pins it).
  Inline is disabled inside another inline; a future phase
  can loosen that once a depth-limiter is in place.
- Unit tests (9 new, behind `--features jit`):
  - `inliner_counts_nodes_correctly` — the node-count
    helper lands the right total for a canonical 5-node
    body.
  - `inliner_rejects_fn_with_call_in_body` — correctness
    preserved via the regular indirect-call path.
  - `inliner_rejects_self_recursion` — `is_trivial_leaf`
    returns false when `current_fn == callee_name`; true
    otherwise.
  - `inliner_rejects_body_exceeding_node_limit` — 11-node
    body is rejected.
  - `inliner_preserves_correctness_for_trivial_leaves` —
    multi-leaf program (`double`, `triple`, `add`) produces
    the correct result end-to-end.
  - `inliner_fires_on_nested_trivial_calls` —
    `inc(negate(3))` works with both levels inlined
    (outer's arg-lowering happens BEFORE merge is
    installed, so there's no ambiguity).
  - `inliner_preserves_fib_correctness` — fib(10)=55;
    inliner correctly bails on the non-leaf body.
  - `inliner_fires_on_simple_arithmetic_leaf` — end-to-end
    sanity for the simplest possible leaf.
  - `inliner_shadows_caller_local_with_same_name` —
    caller's `n` is unchanged after an inline whose callee
    also has a parameter named `n`.
- Verification:
  - `cargo test --locked` — 468 passed (no regression).
  - `cargo test --locked --features logos-lexer` — 469 passed.
  - `cargo test --locked --features jit` — 541 passed (was
    532 before RES-175).
  - `cargo clippy --locked --features logos-lexer,z3,jit
    --tests -- -D warnings` — clean.
  - Manual bench (10-sample p50):
    - leaf_heavy 10M **inliner OFF**: 16.94 ms
    - leaf_heavy 10M **inliner ON**: 10.66 ms → **1.59× speedup**
    - fib(25) ON: ~4.26 ms (unchanged from OFF)
