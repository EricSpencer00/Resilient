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
- **E0021** — trait objects (`dyn Trait`)

Numbers are **sticky**: once assigned, a code is never reused.
If a diagnostic is removed, its code is retired but the docs
page stays as a redirect so external cheat sheets don't silently
break.

## Status

RES-206a shipped the initial registry + docs pages for the first
ten codes. RES-4115 (E-E4) extended the registry to E0011..E0021,
added the `rz explain E00NN` / `rz errors list` CLI subcommands,
migrated the high-traffic typechecker/parser/runtime call sites to
emit their code behind `RESILIENT_RICH_DIAG=1` (byte-identical
output otherwise, so `.expected.txt` goldens stay pinned), and
added two CI-enforced guards:

- `docs_error_registry_generation_smoke.rs` validates every page's
  front matter (`title`, `parent`, `nav_order`, `permalink`) and
  heading against `diag::codes`, and checks this index links every
  registered code in registry order — so the registry and the docs
  site can't silently drift apart.
- `codeless_diagnostic_lint_smoke.rs` fails the build if a new
  `render_*_error` funnel function (or `Diagnostic::new` call site
  outside `diag.rs`'s own tests) is added without a registered code,
  via a shrinking allowlist of the pre-existing legacy call sites
  that still return a bare, codeless `String`.

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
- [E0021 — dyn Trait object-safety violation](./E0021)
