---
id: RES-158
title: `impl Point { fn mag(self) -> Float { ... } }` methods on structs
state: OPEN
priority: P2
goalpost: G12
created: 2026-04-17
owner: executor
---

## Summary
Today `distance(p1, p2)` is the only way to organize struct-related
code. Method syntax `p1.distance(p2)` is immediately more
readable, and matches what users of every modern language expect.

## Acceptance criteria
- Parser: `impl <StructName> { <fn_decl>* }` at top level. Each
  `fn_decl` accepts `self` as the first parameter, typed as the
  enclosing struct.
- Method call: `p.mag()` desugars to `Point$mag(p)` post-resolution.
  Dispatch is static (no vtables — we're nominal).
- `self` is immutable by default. Mutation inside a method mutates
  a local copy unless the call site assigns the result back, same
  as current parameter semantics. Document this clearly.
- Unit tests: method definition, method call, method calling
  another method on the same struct.
- Commit message: `RES-158: impl blocks for struct methods`.

## Notes
- This is sugar, not a dispatch system — it produces exactly the
  same bytecode / JIT output as a free function. No perf
  implications.
- Multiple `impl Point { ... }` blocks allowed, collected at
  resolution time. Same-name method across blocks is a duplicate-def
  diagnostic.

## Log
- 2026-04-17 created by manager
