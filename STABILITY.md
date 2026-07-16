# Resilient Stability Policy

Resilient is **1.0**. As of `v1.0.0` the project makes a
[Semantic Versioning 2.0](https://semver.org/) stability commitment for the
**Stable** surface below: it will not break without a MAJOR version bump and a
deprecation cycle. This document describes exactly what that covers and what
remains **experimental** (and therefore still free to change).

If you are depending on Resilient in production, read this file end-to-end
before upgrading compiler versions.

**This file is the single canonical stability policy.** `docs/STABILITY_POLICY.md`
is the same policy published on the [documentation
site](https://ericspencer.us/Resilient/stability-policy) (GitHub Pages only
serves pages under `docs/`, so it can't just be a symlink to this file); if the
two ever disagree, this file is authoritative and `docs/STABILITY_POLICY.md`
should be updated to match. RES-3510 closed the reconciliation gap between the
two files — see the [CHANGELOG](#changelog) entry below.

---

## TL;DR

- Current status: **1.0** — the **Stable** surface follows SemVer: no breaking
  change without a MAJOR bump and a deprecation cycle. Every such change is
  recorded in the [CHANGELOG](#changelog) below.
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

**"The project" version is `resilient/Cargo.toml`'s `version` field** — this
is the canonical value every release tag (`v<version>`) is cut from (see
`weekly-release.yml` and `docs/RELEASE_AUDIT.md`). The other in-tree
workspace crates that aren't published to crates.io (`resilient-runtime`,
`resilient-span`, `resilient-playground`) are kept in version lockstep with
it — bumped in the same PR whenever `resilient/Cargo.toml` bumps — so "the
workspace version" is a single well-defined string rather than several
independently-drifting ones. This resolves `docs/RELEASE_AUDIT.md` Finding B.

---

## 1.0 Rules

From the `1.0.0` tag onward:

- **The Stable surface (below) does not break without a MAJOR bump.** A
  breaking change to it requires at least one MINOR release that ships the
  replacement alongside a deprecation warning first.
- **The Experimental surface (below) may still change without notice** — it is
  explicitly *not* covered by the SemVer commitment until promoted to Stable.
- **Every breaking change is logged** in the [CHANGELOG](#changelog) section at
  the bottom of this file, with a one-line migration hint where practical.
- Tagged releases (`v1.x.y`) are reproducible snapshots — pin to one if you
  want a fixed target.
- `main` is always green in CI, but only tagged releases carry the SemVer
  guarantee.

---

## Feature Stability

The CLI (`rz --help`) already ships a three-way status vocabulary — `stable`,
`backend-limited`, `experimental` — enforced by
`resilient/tests/it/stability_help_smoke.rs`. This section maps that
vocabulary onto the language/tooling surface:

- **Stable** — see below. Deprecation cycle required before removal.
- **Backend-limited** — fully specified and won't change *on the backends
  that support it*, but only available when a build feature or target is
  present; an unsupported build prints a rebuild hint instead of silently
  degrading. Today that's `--jit` (needs `cargo build --features jit`),
  `--lsp` (`--features lsp`), `--emit-certificate`/`verify-cert`/`verify-all`
  (`--features z3`), and the static FFI registry (`--features ffi-static`).
  Backend-limited is a build-time gate, not a promise that the *design* is
  final — several backend-limited surfaces (Z3 verifier encoding, FFI ABI)
  are also listed Experimental below because their shape is still moving.
- **Experimental** — see below. May change without notice.

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
- **`unsafe` blocks** — required wrapper for volatile MMIO intrinsics; the `unsafe { ... }` syntax and the compile-time gate (calling `volatile_read_*`/`volatile_write_*` outside `unsafe` is an error) are stable.
- **Region annotation syntax** — `region NAME;` declarations, `&[NAME] T` shared-reference parameters, `&mut[NAME] T` exclusive-mutable-reference parameters, and the compile-time borrow check that rejects two `&mut[A]` params with the same label in the same function. Stable; the syntactic alias-rejection rule and diagnostic format are part of the stable surface.
- **Region-polymorphic function syntax** — `fn f<R, S>(…)` with region type parameters in angle brackets; call-site substitution and aliasing check (two callee region params resolving to the same mutable region is a compile-time error). Stable for the V1 single-label inference model.

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
  verifier evolves. The V1 verifier surface is intentionally **state-local**
  (per-function `requires`/`ensures`, single-step `recovers_to`, snapshot
  cluster invariants); reasoning over **traces** — liveness, fairness,
  multi-actor interleavings, refinement — is a V2 capability tracked under
  RES-396 (G22 TLA+ ladder, see [ROADMAP.md](ROADMAP.md)). Don't read V1
  Z3 success as a temporal-property guarantee. Note: `recovers_to` is a
  single-transition postcondition — it does not provide an eventually-holds
  guarantee; multi-step recovery operators are a V2 capability tracked under
  RES-396.
- **Package manager (`resilient pkg`)** — subcommand names, manifest format
  (RES-212), and resolution rules.
- **Language server (`--lsp`)** — request/response shapes beyond stock LSP
  are subject to change; new features land as RES-183/184/190 progress.

If you build on any of the above, expect to rev your code on most releases.

### Planned (not yet implemented)

Documented in SYNTAX.md and the guides as the intended design, but **not
yet built** — the compiler does not accept them today. Do not depend on
them; the syntax and semantics may change before they land.

- **`#[interrupt(name = "…")]` attribute** — intended to register a
  zero-parameter unit function as an ISR, lowering to a
  `__resilient_isr_<NAME>` extern symbol resolved by a runtime weak-alias
  vector table. Not implemented: the parser currently rejects `#[interrupt]`
  (only `#[cfg(...)]` is recognized), and no ISR lowering or vector table
  exists. Landing it needs a native embedded codegen path that can emit
  linkable ISR symbols — the current on-device backend is a bytecode VM,
  which has no such path. Tracked by RES-4025.

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

1. The change is proposed as a GitHub Issue (`RES-NNN`).
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
| 2026-07 | 1.0.0   | Release     | RES-4106: **first stable release.** The Stable surface above is now under a SemVer 2.0 commitment — no breaking change without a MAJOR bump + deprecation cycle. Workspace promoted `1.0.0-rc.1`→`1.0.0`; `rz --version` now reads `stable release`. Pre-1.0 framing retired (F-E6). VS Code extension published at `1.7.0` (decoupled line, E-E3). No feature-list changes vs rc.1. |
| 2026-07 | 1.0.0-rc.1 | Release  | RES-4102: cut the first 1.0 release candidate. Workspace manifests aligned to `1.0.0-rc.1` in lockstep; `rz --version` notice is now version-derived (`release candidate` on the rc line, retiring the hardcoded `pre-1.0` string — F-E6). The VS Code extension version line was decoupled and moved forward to `1.6.0` past the Marketplace `1.5.3` floor (E-E3). No change to the Stable/Experimental feature lists. API may still shift before the final `1.0.0` tag. |
| 2026-07 | 0.2.x   | Docs        | RES-3510: reconciled this file with `docs/STABILITY_POLICY.md` into one canonical policy; added the `backend-limited` tier (already shipped in `rz --help`, RES-3133) to the written policy. No change to the Stable/Experimental feature lists themselves. |
| 2026-04 | 0.1.x   | FFI         | Static-registry FFI landed behind `--features ffi-static` in `resilient-runtime`. Experimental — signatures, type mapping, and registration macros may change (RES-FFI phase 1). |
| 2026-04 | 0.1.x   | FFI         | `extern "lib" { fn name(...) -> T = "symbol"; }` syntax added for dynamically loaded libm-style libraries in the tree walker. Experimental; struct-by-pointer and callbacks tracked as RES-215 / RES-216. |
| 2026-04 | 0.1.x   | live blocks | `live backoff(base_ms=, factor=, max_ms=)` and `live within Nms` clauses formalised (RES-138 / RES-139 / RES-142). Keyword and parameter names experimental. |
| 2026-04 | 0.1.x   | live blocks | Nested `live` escalation rules and telemetry counter names finalised for the tree walker (RES-140 / RES-141). Counter names experimental. |
| 2026-04 | 0.1.x   | CLI         | `resilient --version` / `-V` added; prints the pre-1.0 stability notice pointing here (RES-209). |

New entries go at the top of the table.

---

## Questions?

Open an issue or email [ericspencer1450@gmail.com](mailto:ericspencer1450@gmail.com).
