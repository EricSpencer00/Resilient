---
title: Standards
nav_order: 17
has_children: true
permalink: /standards
description: "How Resilient maps to safety standards: DO-178C avionics, ISO 26262 automotive, IEC 62304 medical devices, IEC 61508 industrial."
---

# Resilient and safety standards
{: .no_toc }

Resilient is not a certified tool, but its features map directly
to specific objectives in the major safety standards. These pages
list the mappings, honestly, with the gaps called out.
{: .fs-6 .fw-300 }

---

## Per-standard mappings

- **[DO-178C (Airborne Software)](standards/do-178c)** —
  for civil avionics applicants targeting DAL A–E.
- **[ISO 26262 (Road Vehicles)](standards/iso-26262)** —
  for automotive teams targeting ASIL A–D.
- **[IEC 62304 (Medical Device Software)](standards/iec-62304)** —
  for medical device teams targeting Class A / B / C software.

## What Resilient is not

Resilient is **not** a certified tool. No tool qualification
dossier exists. No certification body has audited the compiler.
Claiming otherwise would mislead a safety engineer into
building on a foundation that hasn't been laid.

What Resilient *is* is a language designed with certifiability
as a first-order concern. Its features — function contracts,
SMT-LIB2 certificates, Ed25519-signed manifests, static-only
heap, ASCII-only identifiers, deterministic execution — were
chosen knowing that downstream users may eventually defend
the software to a DER, functional safety manager, or IEC 61508
assessor.

For the full per-objective mapping that consolidates all four
standards on a single page, see
[Certification and Safety Standards](../certification).
