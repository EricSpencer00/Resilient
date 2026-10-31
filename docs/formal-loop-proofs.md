# Loop-invariant proofs

With the `z3` feature, Resilient attempts to prove supported loop invariants using signed 64-bit arithmetic. Runtime invariant checks remain enabled in the interpreter and both VM dispatch engines, whether or not a static proof succeeds.

The query uses `RESILIENT_OVERFLOW_MODE` as execution does:

- `wrap` models two's-complement wraparound.
- `saturate` clamps overflowing results to the signed 64-bit range.
- `trap` requires executed arithmetic to avoid overflow.

An accepted query also establishes definedness of the arithmetic it models. Division and remainder require a nonzero divisor. The `MIN / -1` and `MIN % -1` cases follow the selected runtime policy. Boolean short-circuiting and skipped loop bodies do not require unreachable operations to be defined.

## While loops

The primary proof checks the initial invariant and its preservation after a straight-line assignment body. The induction step considers all machine states satisfying the invariant and loop condition, while retaining known constants that the body does not modify. It checks the condition's definedness and all executed assignment expressions, including assignments whose values do not appear in the invariant.

An invariant can hold on a concrete finite execution without being inductive over every state it describes. When induction does not succeed, the prover can check a concrete terminating trace from known entry bindings. This fallback checks each invariant, condition, assignment expression, and final false condition. It expands at most 128 iterations. Unknown branches, unsupported expressions, arithmetic faults, or reaching the expansion limit with an unfinished loop yield no proof credit. Partial traces are not published as successful loop proofs.

## Finite for loops

Literal integer arrays of at most 128 elements are expanded in order. Each actual element is bound before its invariant checkpoint. Assignments retain machine arithmetic semantics through subsequent checkpoints. The final body is checked for arithmetic definedness even though no additional invariant checkpoint follows it. An empty array has no executed invariant checkpoint and receives no invariant proof.

## Certificates and limits

Successful queries declare `QF_BV`, include the selected overflow policy, and retain the exact negated solver claim for independent replay. A successful replay returns `unsat`. Finite while-trace queries identify their proof method separately from induction. Base and step obligations remain separate certificates.

Integer symbols must have a tracked signed-integer domain; floating-point and unknown values are not silently encoded as integers. Function parameters and lexical shadowing are accounted for.

The supported expression model includes signed integer literals and identifiers, arithmetic, signed comparisons, Boolean connectives, and integer bitwise AND/OR/XOR. Unsupported constructs receive no static proof. Query size, expression depth, and solver time are bounded; unknown or timed-out results do not count as proofs. Symbolic substitution charges a shared 32,768-node expansion budget before allocating copies, with a depth limit of 128. Finite traces share their budget across assignments and iterations. Exhausting it declines static proof credit while preserving runtime guards.

These certificates establish the emitted obligations within this restricted model. They do not prove arbitrary compiler correctness, program termination in general, every program safety property, or certification to a safety standard. Runtime guards remain part of the executable behavior.
