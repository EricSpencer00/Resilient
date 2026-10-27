---
layout: page
title: "From Requirements to Assurance"
nav_order: 31
permalink: /assurance-provisions/
---

# From requirements to assurance

Agent-written code can be excellent and can outperform human-written code on a
given task. Neither authorship establishes correctness. If a project is called
“vibe-coded,” ask which requirements and failure modes have supporting evidence.
Apply the same questions to code, specifications and tests written by people,
agents, or both.

## Three connections to check

| Connection | What can go wrong | Evidence to request |
|---|---|---|
| Intended requirements and environment → specification | A precise, internally consistent spec omits a required outcome or assumes away a failure. | Map requirements to properties; review assumptions with domain owners and retain scenarios that challenge the model. |
| Specification → implementation | The model is right, but deployed code takes a different transition or handles arithmetic/failure differently. | Identify the implementation revision and backend; document a refinement mapping or other conformance argument and compare actual execution with model observations. |
| Properties → tests and checks | Checks omit a property, exercise the wrong component, or accept a vacuous assertion. | Record enabled properties, actual execution, coverage limits, negative controls and unresolved outcomes. |

These connections need separate evidence. A conforming implementation can
faithfully implement an incomplete or wrong specification. Tests can pass against
an oracle that repeats the specification's mistake. Artifact hashes and signatures
bind identity; they do not establish these connections.

## What a TLC result establishes

[TLC's model configuration](https://github.com/tlaplus/tlaplus/blob/master/toolbox/org.lamport.tla.toolbox.doc/html/model/overview-page.html)
selects the behavior specification, constant assignments and enabled invariants,
temporal properties and deadlock checks. A completed exhaustive run finding no
violation supplies evidence for those checks on that configured model, subject
to tool assumptions and any constraints or reductions. Report exploration mode
and completion status:
[simulation and depth-limited exploration](https://github.com/tlaplus/tlaplus/blob/master/toolbox/org.lamport.tla.toolbox.doc/html/model/tlc-options-page.html)
provide different coverage from exhaustive exploration.

A TLC pass alone does not prove that all intended requirements were modeled,
that larger or unrestricted systems satisfy the properties, or that executable
code conforms. SANY acceptance checks specification well-formedness rather than
requirements fidelity. A timeout, skipped property or unsupported check leaves
an open obligation.

TLA+ can express refinement relationships between specifications; model checking
and theorem proving can support refinement arguments. Implementation verification
can connect code to a specification through additional methods and obligations.
Those capabilities are real, but a model-checking result alone does not establish
that connection for a particular program. See Lamport's
[*Specifying Systems*](https://lamport.azurewebsites.net/tla/book.html), including
the refinement discussion and TLC chapter.

## Example: a timer that never releases its waiter

Suppose a timer must notify at most once **and** every accepted delay must finish
or report cancellation when its driver keeps ticking. A model records only the
notification count. Checking `notifications <= 1` can pass even if a waiting
operation never returns. Code that follows that model exactly can still miss the
progress requirement.

Extend the model with waiting, completion and cancellation states. Write the
progress property and state the scheduling/driver fairness assumptions. Review
whether those assumptions hold in deployment. Exercise the actual implementation
with a delay longer than a one-shot period and with stop/reset during a wait.
A deliberately frozen countdown should fail the relevant check. If only the
model was checked, keep implementation conformance open. This is an illustrative
review scenario, not a claim that a particular release completed this evidence.

## Provisions for a change or release

1. **Record intended behavior.** Give each critical requirement an identifier,
   owner and source. Include rejected operations, failures and progress alongside
   normal outputs. Check that the source supports the requirement; a citation
   alone is insufficient.
2. **Review the abstraction.** Map requirements to variables, transitions and
   properties. List omissions and assumptions about clocks, scheduling, arithmetic,
   crashes and external services. Challenge overly strong assumptions and
   unreachable antecedents with representative scenarios.
3. **Bind the run.** Retain spec/configuration, tool version, command, constants,
   fairness, constraints, reductions, enabled properties, completion and results.
   Distinguish exhaustive checking, simulation, bounded exploration and theorem
   proofs.
4. **Connect actual code.** Record source/build identity and execution path.
   Review the state/action mapping. Run conformance or differential checks over
   boundaries and failures; state the tested domain. If a refinement proof exists,
   identify its theorem, assumptions, trusted translation and covered code.
5. **Challenge the checks.** Use deliberate faults or negative controls for each
   critical requirement. Confirm the responsible check fails for the intended
   reason. Review missing obligations as well as successful assertions; finite
   tests supply evidence within their scope.
6. **State the remaining claim.** Separate established properties, sampled
   observations and open obligations. Review changed requirements, assumptions,
   code and deployment configuration before carrying evidence forward.

A review record might say: “No violation of `AtMostOnce` in the completed
two-client model at revision X; cancellation progress was not enabled;
implementation comparison covers A–D on backend Y; larger populations and other
backends remain open.” This is more actionable than “formally verified.”

## Reading Resilient's advisory scores

`resilience_score` counts weighted AST signals: contract declarations, purity,
`live` blocks, call references and body size. `vibe_debt` counts missing
precondition, postcondition, call-reference and effect-annotation signals.
Neither score measures author quality, proves contracts, runs tests, validates
the intended specification, or establishes implementation conformance. A call
reference is not execution coverage; a `live` block is not a liveness proof.
Inferred contracts need independent review against requirements because the code
and inferred contracts can share a mistake.

Existing CLI/API names remain compatible; treat thresholds as annotation
policies. Use verifier results and the evidence above for correctness claims.
See [verification limits](VERIFICATION_LIMITS.md) and the
[contract workflow](HOWTO_PROVABLY_CORRECT_AI_CODE.md).

## Public AI4FM research context

The [AI4FM software page](https://ai4fm.cs.luc.edu/pages/software/) links public
TLA+ generation, checking and evaluation artifacts. These support research into
generated specifications and checker feedback. Report the task, configuration
and metric: parser/checker acceptance alone does not establish requirements
fidelity, implementation conformance or theorem-proof success. This guide makes
no claim about unpublished evaluations.
