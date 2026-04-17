---
id: RES-060
title: Constant-fold contract clauses at typecheck time
state: DONE
priority: P1
goalpost: G9
created: 2026-04-17
owner: executor
---

## Summary
**First Phase 4 brick.** The big G9 target — full SMT-backed
discharge of requires/ensures clauses — is months of work. This
ticket delivers the infrastructure by handling the trivial subset:
contract expressions that are constant-foldable.

For the first time, Resilient is **statically verifying** a
contract. No SMT solver needed yet; that comes with G9b.

## Acceptance criteria

- `requires true` → statically discharged (noted but accepted)
- `requires false` → compile-time error: "contract will always fail"
- `requires 5 != 0` → statically discharged (fold to true)
- `requires 0 != 0` → compile-time error
- `requires 5 > 0` → statically discharged
- Non-foldable clauses (like `requires x > 0` where x is a parameter)
  → left untouched, still checked at runtime
- `--typecheck` is the entry point; clean programs pass, contradictions fail
- Three tests: tautology accepted, contradiction rejected, symbolic-x left alone

## Notes
- New helper: `constant_eval(expr: &Node) -> Option<Value>`
  — returns Some(Value) if the expression has no free variables,
    None otherwise.
- Applied to every requires/ensures clause in every Function and
  FunctionLiteral during typecheck.
- Reuses existing Value types so arithmetic and comparison folding
  ride on the interpreter's existing correctness.
- This is explicitly NOT G9 in full — it's the infrastructure brick
  that G9b's real SMT layer will plug into.

## Log
- 2026-04-17 created and claimed
