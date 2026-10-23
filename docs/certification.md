---
title: Certification and Safety Standards
parent: Standards
nav_order: 1
permalink: /certification
---

# Certification and Safety Standards
{: .no_toc }

What Resilient contributes to DO-178C, ISO 26262, IEC 61508, and
MISRA-style coding standards — and what the integrator still owns.
{: .fs-6 .fw-300 }

<details open markdown="block">
  <summary>Table of contents</summary>
  {: .text-delta }
- TOC
{:toc}
</details>

---

## Honest framing

Resilient is **not** a certified tool and is **not**
DO-178C / ISO 26262 / IEC 61508 compliant. No accepted tool qualification
dossier is established here. No certification body has audited the compiler.
Claiming otherwise would mislead a safety engineer into building
on a foundation that hasn't been laid.

What Resilient *is* is a language designed with certifiability as
a first-order concern. Every feature in the language was chosen
with the knowledge that a downstream user may eventually have to
defend the software to a Designated Engineering Representative
(DER), a functional safety manager, or an IEC 61508 assessor.

The path this page describes is the realistic one:

1. Resilient's features can contribute evidence to selected objectives,
   subject to validation of the intended use.
2. Resilient's features **do not eliminate** the need for tool
   qualification, system-level hazard analysis, hardware safety
   mechanisms, requirements management, or independent verification.
3. Until the compiler itself is qualified, Resilient's role in a
   certified product is **as an input artifact generator** — its
   proof certificates, audit reports, and signed manifests feed a
   qualified process run by the integrator.

The rest of this page maps concrete features to concrete objectives
in each standard, and flags the gaps.

### What an `ensures` certificate attests

A postcondition proof is only as strong as what it is proven *against*.
Resilient discharges an `ensures` clause against the **function body**:
the body's return expression is substituted for `result`, so a passing
verdict means the value the function actually computes provably
satisfies the clause for every admitted input, and a failing verdict
carries a concrete counterexample — an input that meets the
preconditions yet returns a forbidden value. Straight-line
(`return E;`) and single `if/else`-of-returns bodies over pure
arithmetic/boolean expressions are proven this way; the per-clause
`"basis"` field in the contract certificate reads `"implementation"`.

Bodies outside that subset (loops, local bindings, calls in the return
position) fall back to a **clause-only** proof, where `result` is a free
variable: a pass then attests that the postcondition is *self-consistent
with the preconditions*, **not** that the implementation honours it. The
certificate labels these `"basis": "clause-only"`, and the retained
runtime check — not the static proof — is what guards them. Do not read
a `clause-only` pass as a verified implementation when assembling
verification evidence.

---

## Standard-specific preparation

- [ISO 26262](standards/iso-26262): intended-use TI/TD evaluation, TCL determination and qualification rationale.
- [DO-178C / DO-330](standards/do-178c): tool criteria, qualification level where applicable, and agreed formal-methods credit.
- [IEC 62304](standards/iec-62304): manufacturer's lifecycle, risk management and intended-use validation.
- [IEC 61508](standards/iec-61508): support-tool assessment and SIL-specific methods justification.

Use the [qualification preparation workflow](qualification-readiness) and its
machine-checkable intake template. It records evidence gaps without assigning
qualification, certification or compliance status. MISRA C is a C coding standard,
not a certification standard for the Resilient language.

### Proof and integrity boundaries

Solver replay checks the emitted formula. It does not independently establish
that the requirements are correct, that source-to-formula translation is sound,
or that compiler lowering and runtime execution preserve the modeled behavior.
Validate those links separately for the selected toolchain and target.

Signatures and hashes protect artifact integrity only within their implemented
coverage and key-trust model. Record source and binary identities separately;
a signed formula is not a signed safety case. Check actual runtime enforcement
for each backend before using deferred contract checks as test oracles.

---

## Generating audit artifacts

The options that generate candidate verification artifacts are
`--audit`, `--emit-certificate`, and `--sign-cert`. They compose when
the `rz` binary was built with `--features z3`:

```bash
mkdir -p ./artifacts/certs
# One shot: typecheck + verify + audit + emit signed certificates.
rz \
    --audit \
    --emit-certificate ./artifacts/certs \
    --sign-cert ~/.resilient-priv.pem \
    resilient/examples/cert_demo.rz \
    > ./artifacts/audit.txt
```

Output layout:

```
artifacts/
  audit.txt                          # human-readable coverage report
  certs/
    manifest.json                    # per-obligation sha256 + sig
    cert.sig                         # Ed25519 over the concatenated .smt2 files
    <fn>__<kind>__<idx>.smt2         # one re-verifiable proof per obligation
    ...
```

Downstream verification (auditor's machine, no Resilient compiler
needed beyond the verifier subcommand):

```bash
# Cryptographic regression check — fast.
rz verify-all ./artifacts/certs

# With re-run of Z3 on every certificate — slower but strongest.
rz verify-all ./artifacts/certs --z3
```

### Incorporating artifacts into project evidence

Link each artifact to its requirement, exact source and binary baseline, tool
configuration, expected result, actual result and review record. Include solver
exit status and complete output; treat unknown, errors and timeouts as unresolved.
An independent reviewer must assess the formalization and translation as well as
replay. Refer to the project's agreed objectives rather than assuming a report
or signature satisfies an entire standards clause.

---

## Roadmap

The gap between "features that contribute to a safety argument"
and "a qualified toolchain" is the honest subject of this
section. The items below are roadmap work only: they are not
current toolchain capability, certification evidence, or a claim of
conformance.

- **Tool qualification dossier (G19+).** An intended-use-specific DO-330 /
  ISO 26262 evaluation of the verifier subset used for
  certificate generation, starting from the Z3 translation layer
  and the signature verification path — the smallest credibly
  qualifiable slice.
- **WCET analysis integration.** Coordinate with an existing
  static WCET tool (aiT, Bound-T) applied to the Cranelift JIT
  output or to a separate AOT backend targeting the supported
  MCUs.
- **MC/DC coverage tooling.** Source-level branch and MC/DC
  instrumentation for the interpreter and bytecode VM backends.
- **Formal memory-safety proof.** Today the runtime inherits
  Rust's safety guarantees; a formal argument (in Coq, Lean, or
  Isabelle) that the Resilient abstract semantics is preserved
  by the bytecode VM and JIT lowerings is a long-term target.
- **Concurrency analysis.** The language is single-threaded
  today. Interrupt-service-routine semantics, lockstep CPU
  support, and a data-race-free concurrency model are open
  research — see the [Concurrency](concurrency) page for the
  current story.
- **Partnership with certification bodies.** Engagement with
  TÜV, SGS, or equivalent is a prerequisite to any formal claim
  of conformance. No such engagement has started.

---

## Further reading

- [Design Philosophy](philosophy) — the verifiability pillar and
  why the language looks the way it does.
- [Memory Model](memory-model) — allocation behavior and the
  `static-only` feature.
- [Concurrency and Real-Time Scheduling](concurrency) — what the
  language currently says (and does not say) about concurrency.
- [GitHub Issues](https://github.com/EricSpencer00/Resilient/issues)
  — the live engineering ledger, including every verification
  and certificate ticket (RES-067, RES-071, RES-131, RES-132,
  RES-136, RES-137, RES-178, RES-194, RES-195).
