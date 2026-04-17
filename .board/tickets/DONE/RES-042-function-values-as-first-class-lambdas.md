---
id: RES-042
title: Function values as first-class (anonymous fn, closures)
state: DONE
priority: P1
goalpost: Phase2
created: 2026-04-16
owner: executor
---

## Summary
Functions currently only exist as top-level declarations. Making them
first-class — passable as arguments, returnable from functions,
storable in variables — is the last Phase 2 brick.

## Acceptance criteria

    let add = fn(int a, int b) { return a + b; };
    println(add(2, 3));            // 5

    fn make_adder(int n) {
        return fn(int x) { return x + n; };
    }
    let add5 = make_adder(5);
    println(add5(10));             // 15

    // Higher-order: pass fn into fn
    fn apply(int v, int x) {
        return x + v;
    }
    // (true HOFs come with Phase 3 type inference; for MVP we
    //  demonstrate the value semantics)

- `fn(params) { body }` parses as an expression.
- The produced value is `Value::Function` identical to named fns.
- Captured environment is by-value (a snapshot at creation time),
  matching the existing Value::Function semantics. Real lexical
  closures with shared mutation land with a later ticket when
  Environment becomes `Rc<RefCell<...>>`.
- Tests: anonymous fn called directly, stored and called, returned
  and called (the adder pattern).

## Log
- 2026-04-16 created and claimed
