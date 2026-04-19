---
title: Error Index
has_children: true
nav_order: 7
permalink: /errors/
---

# Error index
{: .no_toc }

Every diagnostic the Resilient compiler emits gets a stable
code — `E0001`, `E0002`, etc. When you see one in your
terminal or IDE, look it up here for a minimal reproducing
example and the standard fix.
{: .fs-5 .fw-300 }

---

## Layout

Codes are grouped by the pipeline stage that can emit them:

- **E0001..E0003** — parser
- **E0004..E0006** — name resolution
- **E0007** — type checker
- **E0008..E0009** — runtime (interpreter / VM / JIT)
- **E0010** — contracts (`requires` / `ensures`)

Numbers are **sticky**: once assigned, a code is never reused.
If a diagnostic is removed, its code is retired but the docs
page stays as a redirect so external cheat sheets don't silently
break.

## Status

RES-206a shipped the initial registry + docs pages for the ten
codes above. The remaining ~30 existing diagnostic sites still
emit uncoded errors; RES-206b audits each site and assigns a
code, and RES-206c fleshes out the remaining docs pages.

## Browse

See the sidebar for the full list, or jump directly:

- [E0001 — Generic parse error](./E0001)
- [E0002 — Expected / missing `;`](./E0002)
- [E0003 — Unclosed delimiter](./E0003)
- [E0004 — Unknown identifier](./E0004)
- [E0005 — Unknown function](./E0005)
- [E0006 — Call arity mismatch](./E0006)
- [E0007 — Type mismatch](./E0007)
- [E0008 — Division by zero](./E0008)
- [E0009 — Array index out of bounds](./E0009)
- [E0010 — Contract violation](./E0010)
