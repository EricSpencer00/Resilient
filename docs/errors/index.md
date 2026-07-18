---
title: Error Index
has_children: true
nav_order: 9
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
- **E0004..E0006, E0015** — name resolution
- **E0007, E0013, E0016, E0017** — type checker
- **E0008, E0009, E0014, E0018** — runtime (interpreter / VM / JIT)
- **E0010, E0019** — contracts (`requires` / `ensures`)
- **E0011, E0012** — declarations / bindings
- **E0020** — effects / purity

Numbers are **sticky**: once assigned, a code is never reused.
If a diagnostic is removed, its code is retired but the docs
page stays as a redirect so external cheat sheets don't silently
break.

## Status

RES-206a shipped the initial registry + docs pages for the first
ten codes. RES-4115 (E-E4, increment 1) adds a second batch,
E0011..E0020, plus the `rz explain E00NN` CLI subcommand that
renders these same pages in the terminal (`resilient errors list`
prints every registered code).

Most codes above are cataloged and documented but not yet attached
to their originating `Diagnostic` construction site — `E0007` is
the one call site wired so far. Auditing the remaining call sites
(mostly bare `String` errors today, not `Diagnostic`s) and
attaching codes without changing the rendered string shape that
`.expected.txt` goldens pin is the next increment, followed by a
CI lint that fails on a new codeless `Diagnostic` and generating
this directory from the registry instead of hand-authoring it.

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
- [E0011 — Duplicate function definition](./E0011)
- [E0012 — Reassignment of an immutable binding](./E0012)
- [E0013 — Missing return on a code path](./E0013)
- [E0014 — Unwrap of a None optional](./E0014)
- [E0015 — Import target not found](./E0015)
- [E0016 — Generic trait bound not satisfied](./E0016)
- [E0017 — Unknown or missing struct field](./E0017)
- [E0018 — Recursion / stack usage limit exceeded](./E0018)
- [E0019 — Z3 could not prove a contract clause](./E0019)
- [E0020 — Effect/purity violation](./E0020)
