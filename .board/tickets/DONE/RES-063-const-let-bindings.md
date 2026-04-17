---
id: RES-063
title: Track constant let bindings for the verifier
state: DONE
priority: P0
goalpost: G9
created: 2026-04-17
owner: executor
---

## Summary
**Third Phase 4 brick.** RES-061 folds contracts when call arguments
are LITERALS. Real programs almost always pass variables, not
literals — `divide(10, count)`, not `divide(10, 5)`.

This ticket teaches the verifier that immutable `let` bindings to
constant expressions count as constants for the purposes of
contract folding:

    let n = 5;
    fn pos(int x) requires x > 0 { return x; }
    let r = pos(n);     // discharged: n is 5, 5 > 0

    let bad = 0;
    let r2 = pos(bad);  // ← REJECTED at compile time

    let unknown = read_count();
    let r3 = pos(unknown);  // not foldable, runtime check

## Acceptance criteria
- `let n = 5; pos(n)` discharges the contract
- `let bad = 0; pos(bad)` is rejected at compile time
- `let n = 5; n = 7; pos(n)` falls back to runtime (reassignment kills constness)
- Reassignment to a non-constant expression also invalidates
- Tests cover all three paths

## Notes
- TypeChecker grows `const_bindings: HashMap<String, i64>`.
- LetStatement: if value folds via `fold_const_i64`, record the
  constant. Otherwise REMOVE any existing entry (shadowing).
- Assignment and StaticLet: invalidate the binding (mark non-const).
- CallExpression: pass `const_bindings` to the param-fold pass.

## Log
- 2026-04-17 created and claimed
