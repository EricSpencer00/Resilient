---
title: Verify Contracts, Not Provenance
nav_order: 12
permalink: /ai-generated-design
---

# Design: Contract Verification and the `@ai_generated` Provenance Alias

> **History.** This page originally designed `@ai_generated` as a
> verification gate: tagged functions were forced to carry contracts
> and bounded loops. RES-3854 reversed that framing — **correctness is
> not a property of authorship** — and RES-3857/RES-3858 re-landed the
> verification machinery on a provenance-agnostic policy. This page
> documents the current model.

## Why provenance was the wrong axis

The original RES-3780 design hung real guarantees (non-vacuous
contracts, Z3-proved loop bounds) off a `@ai_generated` marker. That
coupling was backwards:

1. **Correctness is not a property of authorship.** A hand-written
   safety-critical function deserves the exact same contract-vacuity
   and loop-bound proofs as one emitted by a model.
2. **The marker is trivially omittable.** A guarantee you can opt out
   of by deleting one line is a documentation convention, not a
   guarantee.
3. **It bifurcated the verifier.** Verification that only fires under a
   provenance flag invites drift between the "AI" path and the normal
   path.
4. **Provenance ≠ trust.** Tagged code you didn't verify is no safer;
   verified code is safe no matter who wrote it.

## The current model

### `@require_contracts` — the policy directive (RES-3854)

A module-level directive enrols **every** function in the file into
contract verification (`resilient/src/contract_policy.rs`):

```resilient
@require_contracts

fn clamp_positive(int x) requires x < 1000 ensures result >= 0 {
    if x < 0 { return 0; }
    return x;
}
```

Under the bare directive, every *declared* clause must be non-vacuous:

- each `requires` must reference at least one parameter
  (`requires true` is a compile error);
- each `ensures` must reference `result`.

### `@require_contracts(strict)` — mandatory contracts

The strict variant additionally mandates contract *presence*: every
named function (except `main`) must declare at least one `ensures`
clause, and at least one `requires` clause when it has parameters.
Safety-critical modules flip this on, and nobody can opt a function
out by simply not writing a contract.

### Tier 2 — bounded loops (RES-3857)

Any enrolled function containing a `while` loop must carry
`#[loop_bound(N)]`; with `--features z3` the compiler proves (or
refutes) the bound for monotonic-counter loops
(`resilient/src/loop_bound.rs`).

### Tier 3 — proof certificates (RES-3859)

`rz <file> --emit-contract-certificate <FILE>` writes a deterministic
JSON audit artifact attesting the per-clause verdict (`pass` with a
replayable SMT-LIB2 dump, `fail` with a counterexample, or `unknown`).
The certificate attests the **proof**; provenance appears only as an
informational field.

### `@ai_generated` — pure provenance metadata (RES-3858)

`@ai_generated` still parses, and is recorded as an audit-trail
provenance alias of the `#[generated(intent=..., prompt_hash=...)]`
annotation (RES-3835). It grants **no** verification behaviour:
adding or removing it changes nothing about what the compiler checks
or proves. It surfaces in:

- proof certificates (`"provenance": ["ai_generated"]`), and
- any tooling reading the `feature_attrs` registry.

For a richer audit trail (intent text, prompt hash), prefer
`#[generated(...)]`.

## Migration

| Before (provenance-gated) | Now (policy-gated) |
|---|---|
| `@ai_generated` forces contracts on one function | `@require_contracts(strict)` forces contracts on the whole module |
| `@ai_generated` + `while` forces `#[loop_bound]` | `@require_contracts` + `while` forces `#[loop_bound]` on every function |
| Delete the tag to skip checks | No per-function opt-out exists |
