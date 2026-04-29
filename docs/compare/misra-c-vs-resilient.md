---
title: Resilient vs MISRA C
parent: Compare
nav_order: 3
permalink: /compare/misra-c-vs-resilient
description: "Resilient vs MISRA C:2012 — compile-time bounds, divide-by-zero safety, and contract proofs as a modern alternative for automotive and industrial embedded teams."
---

# Resilient vs MISRA C:2012
{: .no_toc }

For automotive, industrial, and medical-device teams hitting
the limits of MISRA C:2012 conformance and looking for a
language where the rules are enforced by the compiler, not a
linter.
{: .fs-6 .fw-300 }

<details open markdown="block">
  <summary>Table of contents</summary>
  {: .text-delta }
- TOC
{:toc}
</details>

---

## Honest framing

MISRA C is a coding standard layered on top of C — it does
not change the language. Conformance is checked by static
analyzers (Polyspace, LDRA, Coverity, PC-lint), and rule
violations are flagged but not always prevented.

Resilient is a *different language*. The MISRA-style rules
that matter for safety — no implicit conversions, no `goto`,
no recursion in some profiles, bounds checks, no
divide-by-zero — are not rules layered on top. They are
either prohibited by the type system or proven by the
verifier. There is no "deviation" mechanism because there is
no rule to deviate from.

Migrating from MISRA C to Resilient is a rewrite. It is not a
trivial decision. Most automotive teams will stay on MISRA C
for the foreseeable future. This page exists for teams whose
risk profile or product roadmap justifies a clean break.

## Side-by-side

| Concern | MISRA C:2012 | Resilient |
|---|---|---|
| Substrate | C99 / C11 + ruleset | Original language |
| Rule enforcement | Static analyzer + manual review | Compiler errors, type system, Z3 proofs |
| Implicit conversions | Discouraged (Rule 10.x) | Disallowed by language design |
| `goto` | Restricted (Rule 15.x) | No `goto` in the language |
| Recursion | Restricted in some profiles (Rule 17.2) | Restricted in the verified subset |
| Pointer arithmetic | Restricted (Rule 18.x) | No raw pointer arithmetic |
| Divide-by-zero | Manual review + analyzer | Compile-time linter (RES-133) |
| Array bounds | Manual review + analyzer + runtime | Compile-time bounds checks from contracts |
| Tool qualification | Some analyzers qualified per ISO 26262 | Not qualified |
| Maturity | Industry standard since 1998 (MISRA C:1998) | Research-stage |
| Hiring pool | Very large (automotive, industrial) | Effectively zero |
| Re-use of existing C code | Yes (legacy continuity) | No — rewrite required |

## Where Resilient earns its keep

- **The rule is the language.** Where MISRA bans implicit
  conversions, Resilient does not have implicit conversions
  to ban. Where MISRA restricts pointer arithmetic, Resilient
  does not expose it. The class of bugs MISRA tries to
  prevent in C cannot be expressed in Resilient.

- **Compile-time proofs of arithmetic safety.** RES-133 makes
  divide-by-zero a compile-time error in trivial cases and a
  Z3 obligation when contracts cover the divisor. MISRA C
  treats this as Rule 1.3 / undefined behavior — flagged but
  not prevented.

- **No deviation mechanism needed.** MISRA conformance reports
  list deviations. Resilient does not have rules to deviate
  from in the same way — the verifier either discharges an
  obligation or it doesn't.

## When MISRA C is still the right call

- **You have a working MISRA-conformant codebase.** Rewriting
  is enormously expensive; the right move is usually to keep
  MISRA C and improve the analyzer config.
- **Your toolchain is locked by your customer.** Tier-1
  automotive suppliers often have hard requirements on
  toolchains, and MISRA C is what's accepted.
- **You need certified compiler output today.** A qualified
  C compiler exists; a qualified Resilient compiler does not.

## When Resilient is worth evaluating

- **You're starting a new product** where the legacy continuity
  argument doesn't apply.
- **You're already paying** for a static analyzer that catches
  half the rule classes Resilient eliminates by design — and
  the analyzer's false-positive rate is hurting velocity.
- **Your risk profile justifies a research-language bet** —
  research vehicle, internal tooling, or a clean-slate
  greenfield product where the certification cost is years
  away anyway.

---

## See also

- [Resilient vs Rust for embedded](rust-vs-resilient)
- [Resilient vs Ada / SPARK](ada-spark-vs-resilient)
- [ISO 26262 mapping](../standards/iso-26262)
- [Certification and Safety Standards](../certification)
