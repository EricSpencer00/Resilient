# Resilient Stability Policy

Resilient is **pre-1.0**. This document describes the stability guarantees the
project offers today and the guarantees it will offer once the language hits
1.0.

If you are depending on Resilient in production, read this file end-to-end
before upgrading compiler versions.

---

## TL;DR

- Current status: **pre-1.0** — breaking changes can land at any time, with no
  deprecation cycle. They are always recorded in the [CHANGELOG](#changelog)
  below.
- `resilient --version` prints a one-line reminder of this status.
- Some features are **experimental** and will almost certainly change; others
  are **stable** and already follow a best-effort deprecation path. The lists
  are in [Feature stability](#feature-stability).
- Experimental surface is marked with a `# @experimental` comment convention.

---

## Versioning Intent (post-1.0)

Once Resilient reaches 1.0 the project will follow
[Semantic Versioning 2.0](https://semver.org/):

- **MAJOR** (`X.y.z`) — incremented for backwards-incompatible changes to the
  stable surface area (see below). Requires at least one MINOR release that
  ships the replacement alongside a deprecation warning. Removed items stay in
  the CHANGELOG indefinitely.
- **MINOR** (`x.Y.z`) — backwards-compatible additions: new syntax that does
  not conflict with existing programs, new builtins, new stdlib functions,
  new compiler flags, additional diagnostics. May also promote an experimental
  feature to stable. Never removes anything.
- **PATCH** (`x.y.Z`) — backwards-compatible bug fixes, performance
  improvements, and documentation. No surface-visible changes.

Compiler-internal APIs (the `resilient` crate's Rust types, the VM bytecode
format, the JIT ABI, the `resilient-runtime` crate's non-public items) are
**never** covered by SemVer and may change in any release.

---

## Pre-1.0 Rules

Until the first `1.0.0` tag:

- **Any release can break any program.** The surface area is still being
  designed — expect syntax, keyword spellings, builtin signatures, and
  standard-library shapes to move.
- **Every breaking change is logged** in the [CHANGELOG](#changelog) section at
  the bottom of this file, with a one-line migration hint where practical.
- **No deprecation cycle is required** pre-1.0, but where it's cheap the
  compiler will still emit a warning for one release before removing a
  construct.
- Tagged releases (`v0.x.y`) are reproducible snapshots — pin to one if you
  want a stable target.
- `main` is always green in CI but is not considered stable at any point.

---

## Feature Stability

### Stable (deprecation cycle required before removal)

These features are considered the core of the language and will go through a
deprecation cycle before any breaking change, even pre-1.0:

- **Core syntax**: `let`, `fn`, `if` / `else`, `while`, `match`, `return`
- **Primitive types**: `Int` (i64), `Float` (f64), `Bool`, `String`, `Bytes`
- **Basic control flow**: expression-level `if`, `while` loops, `match`
  expressions on primitives
- **Integer and float arithmetic operators**: `+`, `-`, `*`, `/`, `%`, `==`,
  `!=`, `<`, `<=`, `>`, `>=`
- **Function call syntax** and the `fn name(type arg, ...)` declaration form
- **String/byte literal escape syntax** (`\n`, `\t`, `\\`, `\"`, `\xNN`,
  `\u{NNNN}`)

Where a change is unavoidable, the compiler will emit a deprecation warning
for at least one release before the construct stops compiling.

### Experimental (may change without notice)

These features are useful today but still evolving. Expect breaking changes
with no warning cycle:

- **`live` blocks** — retry / backoff / timeout semantics, escalation rules,
  and telemetry counter names. See tickets RES-138..RES-142.
- **Effect system** — effect annotations, effect polymorphism for higher-order
  functions (RES-193), and the exact spelling of effect names.
- **FFI (`extern` blocks)** — both the tree-walker dynamic-load path and the
  `resilient-runtime` static registry behind `--features ffi-static`. Types
  that can cross the boundary, struct-by-pointer support (RES-215), and
  callback support (RES-216) are all actively changing.
- **Z3 verification** — verifier directives, certificate format, and the
  SMT encoding (especially the overflow-aware encoding tracked by RES-134).
  Runs only with `--features z3`; API shape is expected to change as the
  verifier evolves.
- **Package manager (`resilient pkg`)** — subcommand names, manifest format
  (RES-212), and resolution rules.
- **Language server (`--lsp`)** — request/response shapes beyond stock LSP
  are subject to change; new features land as RES-183/184/190 progress.

If you build on any of the above, expect to rev your code on most releases.

---

## The `@experimental` Convention

There is no attribute syntax yet; experimental surface is flagged with a
comment above the declaration:

```resilient
# @experimental: live-block API may change — see STABILITY.md
fn retry_with_backoff(int attempts) {
    live backoff(base_ms=10, factor=2, max_ms=1000) {
        # ...
    }
}
```

The convention is:

- The comment appears on its own line directly above the declaration.
- It starts with `# @experimental` followed by a colon and a short reason.
- Tooling does not currently enforce this — it's a promise to future readers,
  not a compiler-checked attribute. A proper attribute syntax is tracked for a
  future ticket.

Files in `resilient/examples/` that exercise experimental surface should carry
the marker near the top of the file.

---

## How Breaking Changes Land

1. The change is proposed as a ticket (`RES-NNN`) under `.board/tickets/`.
2. If the affected surface is listed as **stable** above, the ticket must
   include a deprecation plan: which version prints a warning, which version
   removes the construct.
3. The change ships behind a PR that updates this file's CHANGELOG with:
   - The date (YYYY-MM).
   - The version that first contains the change.
   - A one-line description.
   - A pointer to the migration (ticket link or short snippet).
4. `resilient --version` keeps pointing contributors at this document so
   nobody upgrades blind.

---

## CHANGELOG

Chronological log of breaking and stability-relevant changes. Newest first.

| Date    | Version | Area        | Change                                                                                           |
|---------|---------|-------------|--------------------------------------------------------------------------------------------------|
| 2026-04 | 0.1.x   | FFI         | Static-registry FFI landed behind `--features ffi-static` in `resilient-runtime`. Experimental — signatures, type mapping, and registration macros may change (RES-FFI phase 1). |
| 2026-04 | 0.1.x   | FFI         | `extern "lib" { fn name(...) -> T = "symbol"; }` syntax added for dynamically loaded libm-style libraries in the tree walker. Experimental; struct-by-pointer and callbacks tracked as RES-215 / RES-216. |
| 2026-04 | 0.1.x   | live blocks | `live backoff(base_ms=, factor=, max_ms=)` and `live within Nms` clauses formalised (RES-138 / RES-139 / RES-142). Keyword and parameter names experimental. |
| 2026-04 | 0.1.x   | live blocks | Nested `live` escalation rules and telemetry counter names finalised for the tree walker (RES-140 / RES-141). Counter names experimental. |
| 2026-04 | 0.1.x   | CLI         | `resilient --version` / `-V` added; prints the pre-1.0 stability notice pointing here (RES-209). |

New entries go at the top of the table.

---

## Questions?

Open an issue or email [ericspencer1450@gmail.com](mailto:ericspencer1450@gmail.com).
