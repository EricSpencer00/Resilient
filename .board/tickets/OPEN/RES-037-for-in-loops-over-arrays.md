---
id: RES-037
title: `for..in` loops over arrays
state: OPEN
priority: P2
goalpost: G11
created: 2026-04-16
owner: executor
---

## Summary
Iterating an array currently takes the full while/index dance:

    let i = 0;
    while i < len(xs) { println(xs[i]); i = i + 1; }

Add `for x in xs { ... }` as a parser-level desugar to the same shape.

## Acceptance criteria
- `for x in xs { BODY }` parses, with `xs` any array-valued expression
- Semantics: iterate each element in order, binding it to `x` in a
  fresh env frame. Early return from BODY propagates.
- Tests: sum-via-for matches sum-via-while; iteration stops at the
  right length; empty array is a no-op.
- Token::For keyword
- Desugar in the parser (create a WhileStatement + IndexExpression)
  OR add a dedicated ForIn node. Either is acceptable; pick the one
  with the smaller diff.

## Log
- 2026-04-16 created by manager
