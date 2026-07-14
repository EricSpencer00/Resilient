---
title: Stability Policy
nav_order: 11
permalink: /stability-policy
---

# Resilient Stability and Compatibility Policy

This page is the GitHub Pages copy of the canonical stability policy that
lives at [`STABILITY.md`][stability-md] in the repository root. GitHub Pages
only serves files under `docs/`, so this page mirrors that file's content
rather than linking off-site; if the two ever disagree, `STABILITY.md` is
authoritative (RES-3510 closed the gap between them as of 2026-07).

[stability-md]: https://github.com/EricSpencer00/Resilient/blob/main/STABILITY.md

## Overview

Resilient is **pre-1.0**. Any release can currently break any program — the
surface area is still being designed. This document defines what stability
guarantees exist today, what they will become once the language reaches
1.0, and how breaking changes are recorded so nobody upgrades blind.

## Current status

- `resilient --version` / `rz --version` prints a one-line reminder that the
  compiler is pre-1.0.
- Every breaking change lands in the [CHANGELOG](#changelog) at the bottom
  of `STABILITY.md`, with a one-line migration hint where practical.
- No deprecation cycle is *required* pre-1.0, but where it's cheap the
  compiler still emits a warning for one release before removing a
  construct.
- Tagged releases (`v0.x.y`) are reproducible snapshots — pin to one for a
  stable target. `main` is always green in CI but is not itself stable.

## Feature status vocabulary

The CLI (`rz --help`) ships a three-way status vocabulary that this policy
adopts directly (enforced by `resilient/tests/it/stability_help_smoke.rs`,
so CLI text and this policy cannot drift silently):

| Status | Meaning |
|---|---|
| **Stable** | Core of the language. Goes through a deprecation cycle (at least one MINOR release with a warning) before any breaking change, even pre-1.0. |
| **Backend-limited** | Fully specified and won't change on backends that support it, but requires a build feature or target (`--features jit`, `--features lsp`, `--features z3`, `--features ffi-static`, or a specific embedded target triple). Unsupported builds print a rebuild hint rather than silently degrading. This is a build-time gate, not a design-freeze — some backend-limited surfaces are *also* Experimental below because their shape is still moving. |
| **Experimental** | Useful today but still evolving. Breaking changes with no warning cycle. |

### Stable

Core syntax (`let`, `fn`, `if`/`else`, `while`, `match`, `return`); primitive
types (`Int`, `Float`, `Bool`, `String`, `Bytes`); arithmetic/comparison
operators; function declaration/call syntax; string/byte literal escapes;
`unsafe` blocks (the compile-time gate around `volatile_read_*` /
`volatile_write_*`); the `#[interrupt(name = "…")]` attribute (Cortex-M4F and
RV32IMAC targets); `region NAME;` / `&[NAME] T` / `&mut[NAME] T` region
annotations and their same-function alias-rejection check; and
region-polymorphic function syntax (`fn f<R, S>(...)`, V1 single-label
inference).

The full, current list lives in `STABILITY.md` § Stable — this page doesn't
duplicate it item-for-item to avoid the two copies drifting on exactly the
list that matters most.

### Backend-limited

`--jit` (Cranelift JIT, `--features jit`), `--lsp` (`--features lsp`),
`--emit-certificate` / `verify-cert` / `verify-all` (Z3 verifier,
`--features z3`), and the static FFI registry (`--features ffi-static`).

### Experimental

`live {}` blocks (retry/backoff/timeout semantics); the effect system
(`-e->` arrow — parsed today, polymorphic unification not yet implemented,
see [FAILURE_MODEL.md](/failure-model)); FFI (`extern` blocks, both the
tree-walker dynamic-load path and the static registry); Z3 verification
directives and certificate format (state-local V1 surface; trace properties
are a V2 capability); the package manager (`resilient pkg`, see
[MODULE_SYSTEM.md](/module-system)); and the language server beyond stock
LSP request/response shapes.

## Versioning intent (post-1.0)

Once Resilient reaches 1.0 it follows [Semantic Versioning
2.0](https://semver.org/):

- **MAJOR** — backwards-incompatible changes to the stable surface.
  Requires at least one MINOR release that ships the replacement alongside a
  deprecation warning first. Removed items stay in the CHANGELOG
  indefinitely.
- **MINOR** — backwards-compatible additions: new syntax that doesn't
  conflict with existing programs, new builtins/stdlib functions, new
  compiler flags, additional diagnostics. May promote an Experimental
  feature to Stable. Never removes anything.
- **PATCH** — backwards-compatible bug fixes, performance improvements, and
  documentation. No surface-visible changes.

Compiler-internal APIs (the `resilient` crate's Rust types, the VM bytecode
format, the JIT ABI, `resilient-runtime`'s non-public items) are **never**
covered by SemVer.

## How breaking changes land

1. The change is proposed as a GitHub Issue (`RES-NNN`).
2. If the affected surface is Stable, the ticket must include a deprecation
   plan: which version prints a warning, which version removes the
   construct.
3. The PR updates `STABILITY.md`'s CHANGELOG with the date, the version that
   first contains the change, a one-line description, and a pointer to the
   migration.
4. `resilient --version` keeps pointing contributors at `STABILITY.md` so
   nobody upgrades blind.

## The `@experimental` convention

There is no attribute syntax yet; experimental surface is flagged with a
comment directly above the declaration:

```resilient
# @experimental: live-block API may change — see STABILITY.md
fn retry_with_backoff(int attempts) {
    live backoff(base_ms=10, factor=2, max_ms=1000) {
        # ...
    }
}
```

Tooling does not enforce this today — it's a promise to future readers, not
a compiler-checked attribute.

## Security and memory safety

Memory safety guarantees (no use-after-free, no data races, etc.) are not
subject to change except to become stricter. If a bug is found in the safety
checker, the fix ships immediately even if it invalidates previously-valid
code; that is not treated as a breaking change, because safety is paramount.
Report suspected vulnerabilities the same way as any other issue — through
[GitHub Issues](https://github.com/EricSpencer00/Resilient/issues); there is
no separate embargo/security-advisory process at this stage of the project.

## CHANGELOG

The authoritative, continuously-updated CHANGELOG lives in `STABILITY.md` §
CHANGELOG. This page is not re-synced on every entry — check the source file
for the current table.

## Questions?

Open an issue or email [ericspencer1450@gmail.com](mailto:ericspencer1450@gmail.com).
