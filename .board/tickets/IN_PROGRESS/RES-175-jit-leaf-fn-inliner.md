---
id: RES-175
title: Cranelift: inline trivial leaf functions at call sites
state: IN_PROGRESS
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
