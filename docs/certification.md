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
DO-178C / ISO 26262 / IEC 61508 compliant. No tool qualification
dossier exists. No certification body has audited the compiler.
Claiming otherwise would mislead a safety engineer into building
on a foundation that hasn't been laid.

What Resilient *is* is a language designed with certifiability as
a first-order concern. Every feature in the language was chosen
with the knowledge that a downstream user may eventually have to
defend the software to a Designated Engineering Representative
(DER), a functional safety manager, or an IEC 61508 assessor.

The path this page describes is the realistic one:

1. Resilient's features **reduce** the evidence burden for the
   objectives listed in each standard.
2. Resilient's features **do not eliminate** the need for tool
   qualification, system-level hazard analysis, hardware safety
   mechanisms, requirements management, or independent verification.
3. Until the compiler itself is qualified, Resilient's role in a
   certified product is **as an input artifact generator** — its
   proof certificates, audit reports, and signed manifests feed a
   qualified process run by the integrator.

The rest of this page maps concrete features to concrete objectives
in each standard, and flags the gaps.

---

## DO-178C (Airborne Software)

DO-178C governs software in certified civil aircraft. Compliance
is evidence-based: for each objective in Annex A (tables A-1
through A-10, scaled by DAL A–E) the applicant supplies artifacts
showing the objective is satisfied.

### What Resilient contributes

- **Objective A-5 (Verify software architecture).** Function
  contracts (`requires` / `ensures`) express architectural
  invariants directly in the source. When Z3 discharges them, the
  architectural property is proven for all inputs in the contract's
  domain — stronger than test-based architectural verification.

- **Objective A-6 (Verify source code).** The SMT-LIB2 certificates
  emitted by `--emit-certificate` are re-verifiable under stock Z3.
  This matters because it breaks the circular trust problem: an
  auditor does not need to trust the Resilient binary to accept the
  proof — they re-run the `.smt2` file under their own solver and
  confirm `unsat`. The `cert.sig` Ed25519 signature (RES-194) plus
  per-cert `sha256` hashes in `manifest.json` (RES-195) give
  tamper-evidence from the point of emission onward.

- **Objective A-7 (Verify software requirements).** `requires` /
  `ensures` clauses tied to a function provide traceable mappings
  from low-level requirements (LLRs) to code, with source locations.
  The `--audit` subcommand emits a coverage matrix showing which
  clauses are discharged statically versus deferred to a runtime
  check — useful evidence for A-7 and for the MC/DC-adjacent
  objective A-7.9.

- **Objective A-9 (Software configuration management).** Signed
  certificate bundles provide a cryptographic configuration
  identification artifact: a given source program, compiled with a
  given verifier version, produces a byte-identical certificate set
  whose top-level signature binds the bundle to the signer's key.

### What you still need

- **DO-330 tool qualification.** Resilient is not a qualified tool.
  If the compiler, verifier, or runtime are used in a way that
  their output is not independently verified, each must be
  qualified as a Criterion 1 / 2 / 3 tool per DO-330. That is a
  multi-year effort the project has not started.

- **MC/DC coverage.** DO-178C Level A requires Modified
  Condition / Decision Coverage. Resilient does not ship an
  MC/DC instrumentation or coverage tool. A third-party coverage
  harness has to be integrated.

- **Worst-case execution time (WCET).** Timing analysis is out of
  scope. DO-178C Section 6.3.4.f (accuracy and consistency of the
  source code) does not mandate WCET, but any real-time avionics
  function will need an external static or measurement-based WCET
  analyzer applied to the compiled binary.

- **Hardware testing.** Proofs on the source program do not
  substitute for on-target testing. The compiled binary still
  needs to be exercised on representative hardware.

- **Requirements management system.** Resilient does not store,
  baseline, or trace high-level requirements. That is the domain
  of DOORS, Polarion, or an equivalent tool.

---

## ISO 26262 (Automotive — Road Vehicles)

ISO 26262 covers E/E systems in passenger vehicles. The standard
scales evidence by Automotive Safety Integrity Level (ASIL), from
QM (quality-managed, no ASIL) through ASIL A, B, C, to D (highest).

### ASIL decomposition

The evidence Resilient can provide is proportional to the ASIL:

- **ASIL A / B.** Runtime contracts (`requires` / `ensures`
  compiled to runtime assertions) plus the `--audit` report are
  defensible evidence for the software unit verification objectives
  in Part 6, Tables 7 and 8.
- **ASIL C / D.** Runtime checks alone are insufficient. The
  pipeline needs (a) Z3-discharged static proofs, (b) emitted
  SMT-LIB2 certificates, (c) cryptographic signatures binding the
  certificate bundle to the supplier, and (d) an independent
  peer review of contract completeness — i.e. a human confirming
  that the `requires` / `ensures` clauses capture every relevant
  precondition and postcondition of the unit. Resilient provides
  (a), (b), (c); (d) is always a human process.

### What Resilient contributes

- **Part 6, Table 1 (Methods for specification of software safety
  requirements).** Formal notations are highly recommended up to
  ASIL D. `requires` / `ensures` are a first-order formal notation
  embedded in the source.

- **Part 6, Section 6.4.7 (Software unit design principles).**
  The `static-only` feature of `resilient-runtime` is a
  compile-time assertion that the program allocates no heap. This
  directly addresses the ASIL D expectation that dynamic objects
  and memory allocation be avoided; the constraint is enforced by
  the build system, not by coding convention.

- **Part 6, Table 9 (Methods for software unit verification).**
  Formal verification is highly recommended for ASIL C / D. Z3
  proofs plus SMT-LIB2 certificates supply exactly this.

- **Part 8 (Supporting processes — configuration management,
  change management, verification).** Signed certificate bundles
  with per-file SHA-256 hashes are usable as configuration-managed
  verification artifacts.

- **Part 6, Section 8 (Software integration and verification).**
  The deterministic bytecode VM and `--seed <u64>` PRNG pinning
  make integration tests reproducible bit-for-bit, which Part 8
  recommends for regression evidence.

### What you still need

- **System-level hazard analysis (Part 3).** HARA, ASIL
  assignment, and functional safety concept development are
  activities that precede any software consideration. Resilient
  has nothing to say about them.

- **FMEA, FTA, DFA at the system and hardware levels.** Part 4 /
  Part 5 analyses are out of scope.

- **Hardware safety mechanisms.** Lockstep CPUs, ECC RAM,
  watchdogs, and the rest of the ISO 26262 hardware architectural
  metrics are the domain of the silicon supplier and the ECU
  integrator. Resilient assumes a correct execution substrate.

- **Tool confidence level (TCL) evaluation.** Per Part 8, Section
  11, every tool in the toolchain needs a TCL evaluation. The
  Resilient compiler would currently be TCL3 (high confidence
  required, qualification needed). No qualification kit exists.

---

## IEC 61508 (Functional Safety — Industrial)

IEC 61508 is the generic functional-safety standard for
programmable electronic systems in industrial and process
applications; it is the parent of IEC 61511 (process),
IEC 62061 (machinery), and related sector standards. Evidence
scales by Safety Integrity Level (SIL) 1 through 4.

### SIL mapping

- **SIL 1 / 2.** Runtime-checked contracts plus the Resilient
  `--audit` coverage report are sufficient as software verification
  evidence (Part 3, Table A.5 "Software verification").
- **SIL 3 / 4.** Static proofs are required: Z3-discharged
  obligations, exported SMT-LIB2 certificates, and a signed
  certificate chain. Part 3 Annex C "Properties for systematic
  safety integrity" treats formal proof as a Highly Recommended
  technique at SIL 3/4.

### What Resilient contributes

- **Part 3, Table A.4 (Software design and development).**
  Structured programming is Highly Recommended at all SILs.
  Resilient has no `goto`, no macros, no inheritance, no implicit
  conversions, no null, and a single way to declare a function.
  The surface for systematic faults is small by construction.

- **Part 3, Table A.3 (Software architecture design).** Defensive
  programming is Highly Recommended at SIL 3/4. Live blocks
  (`live { ... }` with retry budget and exponential backoff),
  `assert(cond, msg)` with operand values in the diagnostic, and
  automatic state restoration on recoverable error directly
  implement this technique.

- **Part 3, Table A.9 (Software verification).** Static analysis
  is Highly Recommended. Resilient's type checker, lint pass
  (unused variables, dead code), and verifier collectively provide
  the static-analysis evidence.

- **Part 3, Section 7.4.4 (Support tools).** Re-verifiable
  SMT-LIB2 certificates let the assessor confirm the proof under a
  tool (Z3) that already has a track record in formal-methods
  deployments, reducing the trust burden on the Resilient
  compiler itself.

### What you still need

- **Safety lifecycle management (Part 1).** Concept, overall scope
  definition, hazard and risk analysis, overall safety requirements.
- **Random hardware failure analysis (Part 2).** PFH / PFD
  calculations, diagnostic coverage, safe failure fraction —
  hardware territory.
- **Independence of verification.** The assessor will require
  evidence that verification was performed with the independence
  appropriate to the SIL; Resilient emits artifacts but does not
  enforce the process.

---

## MISRA analogy

MISRA C (and MISRA C++) is a coding standard for C in
safety-critical automotive and adjacent contexts. It does not
apply to Resilient — Resilient is a different language — but the
*spirit* of MISRA C is "remove the constructs of C that produce
undefined, unspecified, or implementation-defined behavior." Many
MISRA C rules exist because C allows something that should never
have been allowed in a safety-critical context.

Resilient's design choices align with that spirit by construction:

| MISRA C rule family                     | Resilient's answer                                                                                    |
| --------------------------------------- | ---------------------------------------------------------------------------------------------------- |
| MISRA C Rule 2.4 (no misleading identifiers) | ASCII-only identifiers — rejects homoglyph attacks (Cyrillic `а` vs Latin `a`) at the lexer.     |
| MISRA C Rule 10.x (type conversions)    | No implicit conversions. `int + float` is a type error; coerce with `to_float(x)`.                    |
| MISRA C Directive 4.9 (function-like macros) | No macro system at all.                                                                           |
| MISRA C Rule 14.x / 15.x (control flow) | No `goto`. `if` / `while` only. `live { }` is the single structured retry primitive.                  |
| MISRA C Rule 18.x (pointers and arrays) | No raw pointers in surface syntax. Array access is checked.                                           |
| MISRA C Rule 21.3 (dynamic memory)      | `--features static-only` makes heap allocation a compile error in the embedded runtime.               |
| MISRA C Directive 4.1 (run-time failures) | `requires` / `ensures` contracts with static proof or runtime fallback.                             |
| MISRA C Rule 1.3 (undefined behavior)   | Resilient has no undefined behavior at the language level; the bytecode VM is deterministic.          |

A formal Resilient equivalent of a MISRA-style coding standard —
a numbered ruleset with deviation procedures — is a future
deliverable. For now, the language itself enforces most of what
such a standard would mandate. The integrator's project-level
coding guide only needs to cover what the language *permits* but
the project *disallows* (e.g. limits on `live` block nesting,
naming conventions, file organization).

### Related security standards

- **IEC 62443 (industrial cybersecurity).** ASCII-only
  identifiers are a supply-chain-integrity property: source code
  cannot contain homoglyph-spoofed identifiers that review tools
  might miss. Relevant to IEC 62443-4-1 secure development
  lifecycle requirements for source integrity.

---

## Generating audit artifacts

The three commands that produce certifiable evidence are
`--audit`, `--emit-certificate`, and `--sign-cert`. They compose:

```bash
# One shot: typecheck + verify + audit + emit signed certificates.
resilient \
    --features z3 \
    --audit \
    --emit-certificate ./artifacts/certs \
    --sign-cert ~/.resilient-priv.pem \
    src/main.rs \
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
resilient verify-all ./artifacts/certs

# With re-run of Z3 on every certificate — slower but strongest.
resilient verify-all ./artifacts/certs --z3
```

### Incorporating into a DO-178C Software Accomplishment Summary

A Software Accomplishment Summary (SAS) is the top-level DO-178C
deliverable summarizing how each Annex A objective was satisfied.
Resilient's artifacts map in as follows:

- **Section 3 (Software Life Cycle).** Reference the Resilient
  compiler version and Z3 version used; both are echoed in the
  certificate headers.
- **Section 5 (Software Life Cycle Data).** The
  `artifacts/certs/` directory is a Software Verification Result
  (SVR) item. The `manifest.json` is the index; `cert.sig` is
  the configuration management integrity artifact.
- **Section 7 (Additional Considerations).** If Resilient is
  used as an unqualified tool, its output must be independently
  verified — the `--z3` re-run on every certificate, performed
  on a separate machine by separate personnel, is the
  independent verification.

### Incorporating into an ISO 26262 work product

The ISO 26262 analogue is the Software Verification Report
(Part 6, Clause 9). The certificate bundle + audit log is a
direct input to it. The signed manifest satisfies the Part 8
configuration-management requirement for "unique identification"
of the verification artifact.

---

## Roadmap

The gap between "features that contribute to a safety argument"
and "a qualified toolchain" is the honest subject of this
section. Items below are planned, not delivered.

- **Tool qualification dossier (G19+).** A DO-330 TQL-5 (or
  ISO 26262 TCL3) evaluation of the verifier subset used for
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
