---
id: RES-133
title: `assume(p);` annotation surfaces facts to the SMT context
state: OPEN
priority: P3
goalpost: G9
created: 2026-04-17
owner: executor
---

## Summary
Sometimes the verifier can't see that a precondition holds — maybe
because it came from hardware state we haven't modeled, or because
our encoding is too coarse. `assume(p);` gives the user an escape
hatch: "trust me, this is true; verify other things using it". At
runtime, it's equivalent to `assert(p)` so the trust isn't blind.

## Acceptance criteria
- Parser: `assume(expr);` is a statement.
- Verifier: the predicate is added to the SMT context as an
  asserted fact for subsequent obligations in the same block.
- Runtime: `assume` evaluates exactly like `assert` — if the
  predicate is false, the program halts with
  `assume violated at line:col`. No blind trust.
- `--audit` flag marks assume-site usage clearly (different glyph
  from assert / requires).
- Unit tests: `assume(x > 0); ensure x > 0` proves; violating the
  assume at runtime halts with the right span.
- Commit message: `RES-133: assume() as verifier fact + runtime assert`.

## Notes
- This is a footgun we're deliberately sharpening for verifier
  expressivity. The runtime check keeps the footgun from being
  catastrophic.
- Don't let `assume(false)` silently elide everything — if the
  verifier sees `false` as context, it should warn
  "dead-code region following assume(false)".

## Log
- 2026-04-17 created by manager
