---
id: RES-033
title: `push`, `pop`, `slice` builtins for arrays
state: OPEN
priority: P2
goalpost: G11
created: 2026-04-16
owner: executor
---

## Summary
Arrays (RES-032) are useful today only because `+` concatenates.
Mutating workflows need dedicated ops. Immutable push (returning a new
array) matches the pass-by-value semantics of RES-032.

## Acceptance criteria
- `push(arr, x) -> Array`: returns a new array with `x` appended
- `pop(arr) -> Array`: returns a new array without the last element;
  errors on empty
- `slice(arr, start, end) -> Array`: inclusive start, exclusive end;
  errors on out-of-range bounds
- All three enforce arity and argument types
- `let a = push([1,2], 3)` → `[1, 2, 3]`
- Tests for happy path and error path of each

## Notes
- Add to the BUILTINS slice. Follow the signature pattern of the
  existing math builtins.
- pass-by-value semantics mean push/pop return new arrays; mutating
  push would need Rc/RefCell on Value::Array (separate ticket).

## Log
- 2026-04-16 created by manager
