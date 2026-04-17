---
id: RES-032
title: Array literals and indexing
state: DONE
priority: P0
goalpost: G12
created: 2026-04-16
owner: executor
---

## Summary
Resilient has no way to hold a sequence of values. No program that
processes "the last N sensor readings" or "an array of thresholds"
can be expressed. This unblocks ~every realistic use case.

## Acceptance criteria
- Array literal: `[1, 2, 3]` parses and evaluates to Value::Array
- Indexing: `a[0]` parses and evaluates
- Index assignment: `a[0] = 9;` mutates the array
- `len(arr)` works for arrays (returns int count)
- Mixed-type arrays allowed at runtime (single-type checking lands with G7)
- Out-of-bounds index → runtime error, no panic
- Arrays concatenate via `+`: `[1,2] + [3]` → `[1,2,3]`
- Tests: literal construction, indexing, OOB, len, concat, nested arrays

## Notes
- Need new Token::LeftBracket / Token::RightBracket (`[` / `]`).
- New AST nodes: ArrayLiteral, IndexExpression.
- New Value: Array(Vec<Value>). Needs careful handling in Display/Debug.
- Index assignment is a new statement form; may require extending
  the Assignment Node or adding IndexAssignment.

## Log
- 2026-04-16 created by manager
- 2026-04-16 claimed by executor
