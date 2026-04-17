---
id: RES-061
title: Fold contracts at constant call sites
state: DONE
priority: P0
goalpost: G9
created: 2026-04-17
owner: executor
---

## Summary
**Second Phase 4 brick.** RES-060 folded contracts that had no
parameters. This ticket folds them at *call sites* with concrete
arguments — substituting param names with literal values and folding
the resulting expression.

This is the first time Resilient performs **real symbolic
verification of a real contract**:

    fn divide(int a, int b) requires b != 0 { return a / b; }
    let x = divide(10, 0);   // ← typecheck rejects this NOW

## Acceptance criteria

- `divide(10, 5)` typechecks (10 != 0 is provably true)
- `divide(10, 0)` is REJECTED at typecheck time with a contract-violation message naming the call site
- `divide(10, x)` (x is a free variable) typechecks OK and falls back to runtime check
- The previous RES-060 cases (no-arg contracts) still work
- Tests: 5+ covering accept and reject paths
- The reject error names the function and the failing clause

## Notes
- TypeChecker grows a `contract_table: HashMap<String, (Vec<(String, String)>, Vec<Node>, Vec<Node>)>`
  mapping function name → (parameters, requires_clauses, ensures_clauses).
- check_program first does a pass collecting all top-level Functions
  into the contract table (mirrors the interpreter's hoisting).
- In CallExpression, if the callee is a known Function:
   * fold each argument to a concrete value if possible
   * if all args are constants, build a binding map and fold each
     requires clause with bindings
   * reject if any folds to false

## Log
- 2026-04-17 created and claimed
