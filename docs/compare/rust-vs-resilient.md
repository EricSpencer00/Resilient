---
title: Resilient vs Rust (embedded)
parent: Compare
nav_order: 1
permalink: /compare/rust-vs-resilient
description: "Resilient vs Rust for embedded systems — compile-time contract proofs, no_std runtime, Z3-verified bounds and divide-by-zero safety. Honest comparison for embedded-rust teams."
---

# Resilient vs Rust for embedded systems
{: .no_toc }

A technical comparison for teams already shipping `embedded-rust`
who are evaluating Resilient for tighter compile-time guarantees on
safety-critical firmware.
{: .fs-6 .fw-300 }

<details open markdown="block">
  <summary>Table of contents</summary>
  {: .text-delta }
- TOC
{:toc}
</details>

---

## Honest framing

Rust is a mature, production language with one of the strongest
embedded ecosystems of any modern language. Resilient is a young
research language with a working compiler, JIT, and `no_std`
runtime, but a fraction of Rust's ecosystem and zero certification
history.

If your decision criterion is "ship to a customer in the next 18
months," Rust is the safer choice. If your criterion is "I want
contract proofs at compile time, not just memory safety, and I can
absorb a smaller ecosystem to get them," Resilient is worth
evaluating.

This page does not claim Resilient is better than Rust. It claims
they make different trade-offs.

## Side-by-side

| Concern | Rust (embedded) | Resilient |
|---|---|---|
| Memory safety | Borrow checker — proven at compile time | Static-only heap, no `unsafe`, bounds-checked indexing — proven at compile time |
| Functional contracts | None in core; opt-in via `kani`, `creusot`, `prusti` | First-class `requires` / `ensures`, discharged by Z3 at compile time |
| Divide-by-zero | Runtime panic | Compile-time linter (RES-133) + Z3 proof obligation when contracts cover the divisor |
| Array bounds | Runtime panic on out-of-bounds | Compile-time bounds check from `requires` clauses; runtime guard otherwise |
| Self-healing | Manual `Result` plumbing | `live { }` blocks restore last-known-good state on transient fault |
| Certificates | None native | Re-verifiable SMT-LIB2 certificates, Ed25519-signed |
| `no_std` story | Mature; cargo + crates.io | `resilient-runtime` crate; Cortex-M4F demo passes; smaller ecosystem |
| Cross-compile targets | thumbv7em, thumbv6m, thumbv8m, riscv32, esp32, … | thumbv7em-none-eabihf, thumbv6m-none-eabi, riscv32imac-unknown-none-elf |
| Standards mapping | DO-178C qualification underway via Ferrous Systems / Ferrocene | No qualification dossier; features map to objectives but tool not qualified |
| LSP / editor tooling | rust-analyzer (industry-leading) | `rz --lsp` — diagnostics + hover + goto-def, smaller surface |
| Hiring pool | Large and growing | Tiny (research-stage language) |

## Where Resilient earns its keep

- **Function contracts as proof obligations.** A Rust panic that
  would crash a flight controller is, in Resilient, a Z3 obligation
  resolved at compile time when the surrounding contracts admit
  it. The compiler tells you which assertions became proofs and
  which remained runtime guards. See the [contract proofs example](../language-reference#contracts).

- **Re-verifiable certificates.** Resilient's
  `--emit-certificate` writes SMT-LIB2 files that an auditor
  re-runs under their own Z3. This breaks the circular trust
  problem — the auditor does not have to trust the Resilient
  binary to accept the proof. Rust has no equivalent in-tree.

- **Divide-by-zero and bounds at compile time.** RES-133 surfaces
  trivially-wrong arithmetic at compile time. The same surface
  in Rust panics at runtime unless you reach for `kani` or
  similar third-party tools.

- **`live { }` for transient faults.** Sensor-noise or
  cosmic-ray-bit-flip recovery without manual `Result`-plumbing
  on every operation. Read [live block semantics](../live-block-semantics).

## Where Rust is the right call today

- **You need to ship now.** Rust embedded has battle-tested HAL
  crates, RTOS bindings (`embassy`, `RTIC`), and a hiring pool.
  Resilient does not.

- **You need certified toolchain output.** Ferrocene is on a
  certification path; Resilient is not.

- **You depend on a specific HAL.** If your board's vendor ships
  a Rust HAL, that's where the path of least resistance lives.

- **You need a large team.** Recruiting Rust embedded engineers
  is hard but possible. Recruiting Resilient engineers is, at
  time of writing, not a thing.

## When to pick Resilient

Pick Resilient if **all** of the following are true:

1. You are working on safety-critical or research code where
   compile-time contract proofs would meaningfully reduce risk.
2. You can absorb a smaller ecosystem and the cost of writing
   your own integration glue.
3. You're willing to file issues against a young compiler.
4. You're not on a hard ship deadline that requires a mature HAL.

If any of those four are false, ship Rust today and watch
Resilient for the next 12–24 months.

---

## See also

- [Resilient vs Ada / SPARK](ada-spark-vs-resilient) —
  for the formally-verified-language comparison.
- [Resilient vs MISRA C](misra-c-vs-resilient) —
  for teams migrating from C.
- [Certification and Safety Standards](../certification) —
  what Resilient contributes to DO-178C / ISO 26262 / IEC 61508.
- [no_std Runtime](../no-std) — the embedded story in detail.
