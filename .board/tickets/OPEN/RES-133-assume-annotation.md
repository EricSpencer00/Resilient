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
- 2026-04-17 claimed and bailed by executor (verifier threading +
  missing warning channel)

## Attempt 1 failed

Five pieces bundled:

1. Parser: `assume(expr);` statement → trivial.
2. AST: `Node::Assume { condition, span }` — trivial.
3. Runtime: evaluate like `assert` and halt on false — trivial.
4. **Verifier integration** (the substantive piece): the SMT
   verifier currently proves individual clauses via
   `prove_with_certificate_and_counterexample(expr, bindings)`.
   Carrying an "assumed fact" so subsequent obligations in the
   same block see it requires threading an assumption context
   through `check_node` (for every descend into a Block / fn
   body that might contain `assume`) and out to the verifier
   signature. That's a call-graph-wide change touching every
   site that currently invokes `z3_prove_with_cert`.
5. **`assume(false)` dead-code warning**: the typechecker has
   no warning channel today (surfaced in RES-129's bail). Adding
   one is its own infrastructure ticket — same shape as the
   RES-119 Diagnostic type that's also bailed.

Pieces 1–3 would land in ~50 lines. Piece 4 is a pass-through
refactor that's its own iteration. Piece 5 waits on the warning
channel. The `--audit` glyph tweak (in the acceptance criteria)
is small but depends on piece 4 being in place.

## Clarification needed

Manager, please sequence:

- RES-XXX-a (new): typechecker warning channel.
  `check_program_with_source` returns `(Result<Type, String>,
  Vec<Warning>)` or equivalent. Shared with RES-129 which
  flagged the same gap.
- RES-133a: parser + AST + runtime (pieces 1–3 above). No
  verifier wiring. Runtime `assume` is just `assert` with a
  different error prefix — independently useful for users who
  only run without Z3.
- RES-133b: verifier context threading — add an assumption
  stack to the typechecker, plumbed into `z3_prove_with_cert`
  calls. Includes the `--audit` glyph.
- RES-133c: `assume(false)` dead-code warning via RES-XXX-a's
  channel.

Without that split, the ticket is a multi-iteration undertaking
that would mostly duplicate RES-129's infra requirement. No
code changes landed.
