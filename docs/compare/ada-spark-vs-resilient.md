---
title: Resilient vs Ada / SPARK
parent: Compare
nav_order: 2
permalink: /compare/ada-spark-vs-resilient
description: "Resilient vs Ada/SPARK for safety-critical embedded systems — formal verification, contract proofs, and certifiability compared honestly for avionics, defense, and rail teams."
---

# Resilient vs Ada / SPARK
{: .no_toc }

Two formally-verified embedded languages, very different
maturities. A side-by-side for teams in avionics, defense, rail,
or industrial automation evaluating SPARK against Resilient.
{: .fs-6 .fw-300 }

<details open markdown="block">
  <summary>Table of contents</summary>
  {: .text-delta }
- TOC
{:toc}
</details>

---

## Honest framing

Ada has been used in DO-178B/C-certified avionics and rail
control systems for over thirty years. SPARK 2014 is the
verifiable subset, with a mature toolchain (GNATprove) and a
demonstrated track record of high-DAL deployments.

Resilient does not match SPARK on maturity, ecosystem, or
certifiability today. It does match SPARK in one specific area:
*proof obligations for function contracts discharged by an SMT
solver at compile time*. Where it differs is in the rest of the
language design — modern syntax, JIT-able semantics for dev-time
iteration, and signed certificates as a first-class artifact.

If you can use SPARK and your team already knows Ada, use SPARK.
The rest of this page is for teams who can't, or whose criteria
weight modern tooling over ecosystem maturity.

## Side-by-side

| Concern | SPARK 2014 | Resilient |
|---|---|---|
| Lineage | Ada subset, GNAT/GNATprove | Original language, Rust-implemented compiler |
| Verifier | GNATprove (CVC4, Z3, Alt-Ergo) | Z3 in-tree, SMT-LIB2 certificates re-verifiable under any solver |
| Contract syntax | `Pre =>`, `Post =>`, `Loop_Invariant` | `requires`, `ensures` (modern, terse) |
| Memory model | Heap by default; SPARK subset constrains | Static-only heap by default; no `unsafe` |
| Concurrency | Ravenscar / Jorvik profile | `live { }` supervised execution; concurrency primitives WIP |
| Certificate emission | GNATprove proof reports | SMT-LIB2 + Ed25519-signed `cert.sig` + `manifest.json` |
| `no_std` story | N/A — Ada has its own runtime model | First-class `#![no_std]` runtime crate |
| Cross-compile | Ada cross-compilers, broad coverage | thumbv7em, thumbv6m, riscv32 |
| Certification track record | DO-178B/C up to DAL A, ISO 26262 ASIL D | None — features map to objectives but tool not qualified |
| Hiring pool | Small but stable (defense, avionics) | Effectively zero today |
| License | GPL with exception (or commercial) | MIT |
| Iteration speed | AdaCore commercial cadence | Open-source, ticket-by-ticket |

## Where Resilient overlaps with SPARK

- **Function contracts as proof obligations.** Both languages
  treat `requires` / `ensures` (or `Pre` / `Post`) as obligations
  the verifier discharges before the program is admitted. The
  user-facing experience is similar.

- **Static heap discipline.** SPARK forbids most heap
  allocation by default, as does Resilient.

- **Decidable subset by design.** Both languages choose
  semantics that an SMT solver can decide for the common cases —
  no exception machinery in the verified subset.

## Where the two differ

- **Re-verifiable certificates.** Resilient's SMT-LIB2 output
  + Ed25519 signature is intended to make a third-party
  re-verification trivial. SPARK's report is more deeply
  integrated with GNATprove's evidence model. The Resilient
  approach is lighter weight but has not been through a real
  certification audit.

- **Modern syntax surface.** Ada is verbose by design —
  arguably an advantage for safety-critical readability.
  Resilient is closer to Rust syntax. Pick whichever your team
  reads more fluently.

- **JIT-able semantics.** Resilient runs on a tree-walker, a
  bytecode VM, or a Cranelift JIT. The same source executes
  on a dev laptop or cross-compiles to bare metal. SPARK does
  not have a comparable dev-time iteration story.

- **License.** Resilient is MIT; AdaCore SPARK is dual-licensed
  with a commercial offering for certified use. For a
  research project or open-source product, Resilient is the
  cheaper path.

## When to pick SPARK

- You're shipping a DAL A/B avionics product on a hard
  timeline.
- You're in defense, rail, or nuclear, where Ada/SPARK is the
  default and your customers already accept it.
- You need a qualified compiler today.
- You have an Ada team already.

## When to pick Resilient

- You're starting a new safety-critical-adjacent project where
  you can absorb research-language risk for compile-time
  contract proofs.
- You want re-verifiable certificates as an artifact in your
  build pipeline.
- You're comfortable with a smaller ecosystem and writing
  your own glue.
- You want a single language for dev-laptop iteration and
  bare-metal deployment.

---

## See also

- [Resilient vs Rust for embedded](rust-vs-resilient)
- [Certification and Safety Standards](../certification)
- [Language Reference](../language-reference) — contract syntax
  and verifier behavior.
