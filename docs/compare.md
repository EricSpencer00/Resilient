---
title: Compare
nav_order: 16
has_children: true
permalink: /compare
description: "How Resilient compares to Rust embedded, Ada/SPARK, and MISRA C for safety-critical embedded systems."
---

# How Resilient compares to other safety-critical languages
{: .no_toc }

Side-by-side technical comparisons for teams evaluating Resilient
against established safety-critical toolchains.
{: .fs-6 .fw-300 }

---

## Pick your starting point

If you are already shipping safety-critical embedded code, you have
an existing toolchain. The pages below frame Resilient honestly
relative to what you already use — including where Resilient is
*not* the right choice today.

- **[Resilient vs Rust for embedded](compare/rust-vs-resilient)** —
  for `embedded-rust` teams who want compile-time contract proofs,
  not just memory safety.
- **[Resilient vs Ada / SPARK](compare/ada-spark-vs-resilient)** —
  for avionics, defense, and rail teams comparing two formally
  verified embedded languages.
- **[Resilient vs MISRA C](compare/misra-c-vs-resilient)** —
  for automotive and industrial teams hitting the limits of MISRA
  C:2012 conformance.

## What gets compared

Each page is structured the same way so you can scan them quickly:

| Section | What you'll find |
|---|---|
| **Memory model** | Heap policy, `no_std` story, stack discipline. |
| **Verification** | What's proven at compile time vs runtime. |
| **Toolchain maturity** | Honest framing of certification readiness, ecosystem, hiring pool. |
| **When to pick** | The decision rule we'd use ourselves. |

For a deeper dive into Resilient's verifier and certificate
emission, see the [Language Reference](../language-reference) and
[Certification mapping](../certification).
