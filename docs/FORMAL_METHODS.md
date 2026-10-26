---
layout: page
title: Formal-methods coverage
nav_order: 32
permalink: /formal-methods/
---

# Formal-methods coverage

Resilient has several verification mechanisms with different proof boundaries.
A successful check establishes the property encoded by that mechanism, subject
to its model and assumptions. It is not a proof of the entire application or
compiler. This inventory describes the checked-in implementation; it does not
claim a fresh solver run or safety certification.

## What is modeled

| Mechanism | Evidence in the repository | Boundary |
|---|---|---|
| Z3 function contracts | `resilient/src/contract_verify.rs`, `contract_verify/symbolic_eval.rs`, `verifier_z3.rs` | Supported clauses and function bodies; inspect `implementation` versus `clause-only` proof basis. `Unknown` is not a proof. Requires the Z3 build feature. |
| Loop invariants | `resilient/src/verifier_loop_invariants.rs` | Initiation and preservation for supported body shapes. Unsupported forms do not become proofs. An invariant alone does not establish termination. |
| Lean expression semantics | `resilient/lean-spec/Resilient/`, `resilient/src/lean_spec.rs` | Core expression and signed quotient/remainder lemmas over unbounded integers; emitted modules need explicit Lean checking. Paired runtime examples are bounded evidence, not backend refinement. |
| TLA+ actor runtime | `resilient-runtime/tla/runtime.tla`, `runtime.cfg` | Handwritten scheduler abstraction: 3 identities, 2 payloads, mailbox depth 2, at most 4 sends and 3 crashes. These finite bounds do not establish correctness for all runtime executions. |
| `@refines` annotation | `resilient/src/tla_refines.rs`, [actor model](../TLA_ACTOR_MODEL.md) | Mapping validation and an extern-contract guard. Annotation acceptance does not establish behavioral refinement of the function to the named TLA+ action. |
| Certificate replay | [Tooling](../tooling) | Rechecks the encoded solver query. A valid query proof still needs evidence that the query faithfully represents the source, arithmetic, and required obligations. |

## Remaining modeling gaps

**Source and machine arithmetic correspondence.** Specify overflow, signed
division/remainder, conversions, and exceptional outcomes for each execution
backend. Link the source semantics to the solver encoding and emitted code.
The existing qualification work tracks these boundaries in
[#4952](https://github.com/EricSpencer00/Resilient/issues/4952) and
[#4953](https://github.com/EricSpencer00/Resilient/issues/4953).
A contract proof over mathematical integers cannot by itself establish the
same contract for bounded machine arithmetic.

**Lean theorem generation.** The emitter validates the supported int/bool
fragment and matches the model's 0/1 boolean representation; the typed example
checker elaborates generated theorems explicitly. Division/remainder
results include a definedness domain and `none` for zero or undefined
operands. Signed operations truncate toward zero, with paired interpreter/VM
examples covering signs and representable boundaries. Model additional
parameter types and machine overflow, and prove general backend correspondence. Extend the model and lowering
together for statements, calls, loops, and effects. Then prove correspondence for compiler
transformations and backend execution. See [Lean semantics](../lean-spec/).

**Actor lifecycle and progress.** The current TLA+ actions cover spawn, send,
receive, block and crash. Explicit deregistration, timer expiry, cancellation,
shutdown, hardware interrupts, and OS-thread interleavings need separate
models or an explicit exclusion from the claimed scope. `NoLostMessages` is
an accounting invariant that permits queued and crash-dropped messages; it
is not eventual delivery. `Spec` has no fairness conjunct. `RestartConverges`
constrains a bounded restart counter; it does not prove eventual service
recovery. Model actual progress requirements with explicit environment and
fairness assumptions before claiming liveness.

**Model-to-implementation drift.** The scheduler model is handwritten.
Matching state names and passing Rust tests do not prove a simulation relation.
Changes to `actor_runtime.rs` or `supervisor_runtime.rs` do not currently trigger
the TLA workflow's relevant-file filter. The workflow also permits a successful
skip when the TLC jar cannot be downloaded. Retain whether checking actually
ran, the bounds and solver version, and the checked source revision with each
result. A green workflow alone is insufficient evidence.

**Memory, peripherals and recovery.** The Lean expression evaluator and actor
model do not cover allocator behavior, pointer aliasing, DMA, interrupt timing,
persistent state, or hardware fault assumptions. These need scoped semantics
and application-specific assumptions before they can support end-to-end safety
claims. The `no_std` build constraint is useful build evidence, not a formal
memory-safety or hardware-correctness proof.

## Documentation verification

`docs/verify_tutorial_snippets.sh` executes only fences tagged `resilient` in
`docs/tutorial/*.md`, with a supplied or discoverable compiler. It does not
execute adjacent shell commands, compare displayed output, or verify that Z3
certificates were emitted and replayed. No workflow currently invokes this
script. Tutorial execution and proof verification must be reported separately.

For generated Lean modules, run:

```bash
python3 docs/verify_lean_spec.py --compiler /absolute/path/to/rz
```

This checks two demo expression theorems explicitly after building the model.
`python3 docs/verify_lean_types.py --compiler /absolute/path/to/rz` additionally
checks 18 typed arithmetic/Boolean examples. These are regression examples,
not a universal emitter-correctness theorem.
`python3 docs/verify_lean_domains.py --compiler /absolute/path/to/rz` checks
16 additional partial-arithmetic theorems over all parameter values, including
the undefined result when a divisor is zero.
`python3 docs/verify_lean_signed.py --compiler /absolute/path/to/rz` pairs
113 model observations with both interpreter and bytecode VM runs in wrap
mode. It excludes unrepresentable `i64::MIN / -1` division and does not prove
universal correspondence or other overflow policies.

Its lightweight regression checks run with:

```bash
python3 -m unittest discover -s docs/tests -v
```

A useful documentation evidence record includes the source revision, compiler
features and version, solver versions, exact commands, exit statuses, and
expected artifacts. The qualification umbrella
[#4954](https://github.com/EricSpencer00/Resilient/issues/4954) tracks the broader
assurance work. Passing examples is one part of that evidence, alongside
negative cases that confirm unsupported or incorrect programs are rejected.
