# Resilient Language Reference

## Feature Tier Classification

Resilient features are classified into three tiers that define stability guarantees, deprecation policies, and adoption recommendations for users and library developers.

### Stable

Features in the **Stable** tier are:
- Fully specified in the language reference
- Tested across all supported backends (interpreter, VM, JIT, embedded targets)
- Expected to remain compatible across minor versions
- Safe for users to depend on long-term in production systems
- Guaranteed to work on Cortex-M, RISC-V, and other supported embedded targets

**Graduation criteria:**
- Must have comprehensive test coverage (≥80%)
- Must work identically across all backends
- Must be documented in this reference
- Must have at least 2 minor releases with no breaking changes
- Must have no open design questions (RES-* issues resolved)

---

### Backend-Limited

Features in the **Backend-Limited** tier are:
- Specified and functional on certain backends only
- Documented with explicit backend compatibility tables
- May change in minor versions if backend support changes
- Safe to use if your deployment targets a backend where the feature is stable

**Graduation criteria:**
- Must be fully specified and consistent within the supported backend(s)
- Must have dedicated backend tests that pass
- Must be clearly documented with backend compatibility table
- Can graduate to Stable once all backends implement it

**Examples:**
- JIT-specific optimizations
- RISC-V-specific interrupt handling
- Cortex-M-specific memory-mapped register mappings

---

### Experimental

Features in the **Experimental** tier are:
- Under active design or implementation
- May change significantly between releases, including breaking changes
- Not recommended for production use
- Provided for early feedback and research

**Graduation criteria:**
- Must have an associated RES-* issue describing the design
- Should have at least one end-to-end example
- Requires an issue update every 2 releases explaining status
- Can graduate to Backend-Limited or Stable once design is finalized and implementation is complete

---

## Feature Stability Policy

### Deprecation

When a Stable feature must be removed or significantly changed:
1. Announce deprecation in release notes (minor version bump)
2. Provide clear migration path in compiler diagnostics
3. Support deprecated feature for at least 3 minor versions
4. Remove in next major version

Experimental and Backend-Limited features may be removed or significantly changed without deprecation notice, though RES-* issues should be updated to explain the change.

### Breaking Changes

The following constitute breaking changes to Stable features and require a major version bump:
- Removing a language construct or keyword
- Changing the type or behavior of a built-in function
- Changing error codes or diagnostic messages in a non-backward-compatible way
- Removing support for a previously-stable backend target

The following are NOT breaking changes (minor version bump is sufficient):
- Adding new features
- Making the type system stricter (previously accepted code may now error)
- Adding new compiler warnings
- Improving diagnostic messages

---

## Current Feature Classification

This table is evidence-based: a row is marked **Stable** only if `STABILITY.md`
enumerates it under "Stable (deprecation cycle required before removal)" **or**
it is wired into the typechecker's mandatory extension-pass pipeline
(`resilient/src/typechecker.rs`) and backed by a non-trivial regression-example
corpus (`resilient/examples/`). Anything with parser scaffolding but no
enforcement is marked **Experimental** with an explicit "unenforced" note —
never Stable. File/line references below were verified against
`resilient/src/typechecker.rs` and `resilient/src/lib.rs` at the time of
writing and will drift as the source moves; treat them as pointers, not pins.

| Feature | Tier | Evidence | Notes |
|---------|------|----------|-------|
| Core syntax (`let`, `fn`, `if`/`else`, `while`, `match`, `return`) | Stable | `STABILITY.md` § Stable | Deprecation cycle required before removal. |
| Primitive types (`Int`, `Float`, `Bool`, `String`, `Bytes`) | Stable | `STABILITY.md` § Stable | — |
| Arithmetic/comparison operators (`+ - * / % == != < <= > >=`) | Stable | `STABILITY.md` § Stable | — |
| Function declaration/call syntax | Stable | `STABILITY.md` § Stable | — |
| String/byte literal escapes (`\n`, `\t`, `\xNN`, `\u{NNNN}`) | Stable | `STABILITY.md` § Stable | — |
| `unsafe` blocks (volatile MMIO gate) | Stable | `STABILITY.md` § Stable | Required wrapper for `volatile_read_*`/`volatile_write_*`. |
| `#[interrupt(name = "…")]` attribute | Stable | `STABILITY.md` § Stable | Stable for Cortex-M4F and RV32IMAC targets. |
| Region annotation syntax (`region NAME;`, `&[NAME] T`, `&mut[NAME] T`) | Stable | `STABILITY.md` § Stable | Compile-time same-function alias rejection. |
| Region-polymorphic functions (`fn f<R, S>(…)`) | Stable | `STABILITY.md` § Stable | V1 single-label inference model; call-site aliasing check. |
| Generics (type params + trait bounds, monomorphization) | Stable (evidence-based; not yet in `STABILITY.md`) | `generics.rs`, `generic_structs.rs`, `generic_enums.rs`, `generic_variance_call_sites.rs`; dispatched from `typechecker.rs` (`crate::generics::check`, `crate::generic_enums::check`, `crate::generic_structs::check`); 20 example files under `resilient/examples/generic_*` | Implemented and mandatory-pass-wired, but **`STABILITY.md`'s "Stable" list does not currently name generics** — this is a documentation gap, tracked as part of `RES-3510` (reconcile `STABILITY.md`/`docs/STABILITY_POLICY.md`), not evidence the feature is unstable. |
| Sum types / enums (`enum`, tuple + named payload patterns) | Stable (evidence-based; not yet in `STABILITY.md`) | `sum_types.rs` (`parse_enum_decl`, payload-pattern parsing wired from `lib.rs`); 14 example files under `resilient/examples/enum_*` | Same `STABILITY.md` documentation gap as generics. |
| Exhaustiveness checking (`match` on enums/structs) | Stable (evidence-based; not yet in `STABILITY.md`) | `enum_exhaustiveness.rs`, `struct_exhaustiveness.rs`, dispatched from `typechecker.rs`; `match_struct_exhaustive`/`match_struct_nonexhaustive`/`enum_payload_exhaust_missing` examples | Same `STABILITY.md` documentation gap. |
| Nominal-look traits (`trait`/`impl Trait for T`, structural enforcement) | Stable (evidence-based; not yet in `STABILITY.md`) | `traits.rs`, dispatched from `typechecker.rs` (`crate::traits::check`); 14 example files under `resilient/examples/trait_*` | Dispatch is via the existing `<TypeName>$<method>` mangling — there is no vtable (see Trait objects, below). |
| Default trait method bodies | Stable (evidence-based; not yet in `STABILITY.md`) | `default_trait_methods.rs`, `mod default_trait_methods` in `lib.rs` | `traits.rs`'s own module doc still lists this as "out of scope" — that comment is **stale**; the feature shipped in a later file. |
| Blanket impls (`impl<T: Bound> Trait for T`) | Stable (evidence-based; not yet in `STABILITY.md`) | `blanket_impl.rs`, dispatched from `typechecker.rs` (`crate::blanket_impl::check`, gated on `markers.has_blanket_impl`) | Same stale-comment caveat as default trait methods. |
| `@require_contracts` module directive | Experimental | `STABILITY.md` does not list it; RES-3854 | Enrols every function in the file into non-vacuous-contract and loop-bound verification; `(strict)` additionally mandates contract presence. See [How-To: Provably Correct AI Code](HOWTO_PROVABLY_CORRECT_AI_CODE.md). |
| `@ai_generated` function attribute | Experimental | RES-3858 | Pure provenance alias of `#[generated]`; records audit metadata, grants no verification behaviour. See [How-To: Provably Correct AI Code](HOWTO_PROVABLY_CORRECT_AI_CODE.md). |
| `live` blocks (retry/backoff/timeout) | Experimental | `STABILITY.md` § Experimental | Keyword spellings and telemetry counter names may change without notice (RES-138..142). |
| Effect system (`-e->` effect arrow, `: effect` bound) | Experimental — parsed, **not enforced** | `STABILITY.md` § Experimental; parser support in `lib.rs` (`parse_optional_return_type`, `parse_optional_type_params`, RES-193/RES-775) | The parser records the effect var in a sibling map, but effect-polymorphism unification against a higher-order parameter's actual effects is **not implemented** — the parser comment states it waits on "the prereq chain (HM walker, generics)". Do not treat effect annotations as checked today. |
| Associated types (`type Name;` in `trait`, `type Name = ConcreteType;` in `impl`) | Experimental — enforced | `traits.rs` (`AssociatedTypeDecl`, binding-completeness + `where T::Assoc: Bound` projection checks at call sites, RES-2695); `associated_types.rs` (unknown/duplicate binding detection, A-E3/RES-3933); `typechecker.rs` (`current_self_assoc_types` — `Self::AssocName` resolves to the impl's concrete binding and participates in real return/parameter-type checking, A-E3/RES-3933) | `traits.rs`'s own module doc text is now stale ("Status: In scope for phase 2. Not yet implemented" predates RES-2695 and A-E3 — both shipped). `Self::AssocName` in a trait's *default* method bodies and `T::AssocName` at an arbitrary generic-body use site (as opposed to a `where` bound) remain unresolved — tracked in [#4067](https://github.com/EricSpencer00/Resilient/issues/4067). The `resilient/examples/trait_associated_types_design.rz` file predates all of this, uses unsupported `name: type` field/param syntax, and fails to parse — it is not part of the tested example corpus (empty `.expected.txt`, kept only as a historical design note). See `resilient/examples/trait_associated_type_projection*.rz` / `trait_associated_type_*_reject.rz` for the current, tested behavior. |
| Trait objects (`dyn Trait` type-checking) | Experimental (checking only; no vtable) | `dyn_trait.rs` (RES-4068): `dyn Trait` parses and typechecks — unknown-trait rejection, coercion checking at struct-literal call/let sites (a value coerces to `dyn Trait` only if its concrete type provably implements `Trait`), and method-call resolution against the trait's declared methods | Conservative v1: object-safety checking, vtable construction/codegen across the tree-walker/VM/JIT, and `dyn Trait` in generic/container position are deferred — see the follow-up issue linked from #4068. Dispatch at runtime is still the existing static `<TypeName>$<method>` mangling; `dyn` changes only what the type checker accepts. |
| Region *inference* (implicit, unlabeled reference regions) | Unimplemented / descoped for now | `region_inference.rs` (RES-394: region-variable machinery, union-find table) exists but `typechecker.rs` explicitly notes (RES-1611): `region_inference::infer` is a no-op stub (`Ok(())`) — "the real region-aliasing logic lives in `check_call_site_region_aliasing`", a different, already-Stable code path | Do not confuse this with the Stable "region annotation syntax" row above, which covers *explicit* `region NAME;` labels and their aliasing check — that part ships today. Full unlabeled inference does not. |
| FFI (`extern` blocks, static + dynamic) | Experimental | `STABILITY.md` § Experimental | Both tree-walker dynamic-load path and `resilient-runtime` static registry (`--features ffi-static`) are actively changing; struct-by-pointer (RES-215) and callbacks (RES-216) not final. |
| Z3 verification (`--features z3`, verifier directives) | Experimental | `STABILITY.md` § Experimental | V1 surface is state-local (per-function `requires`/`ensures`, single-step `recovers_to`); trace properties (liveness, fairness, refinement) are V2, tracked under RES-396. |
| Package manager (`resilient pkg`) | Experimental | `STABILITY.md` § Experimental | Subcommand names, manifest format (RES-212), and resolution rules may change. |
| Language server (`--lsp`) | Experimental | `STABILITY.md` § Experimental | Request/response shapes beyond stock LSP subject to change (RES-183/184/190). |
| JIT backend (as a whole) | Backend-Limited | `resilient/src/jit_backend.rs`, `jit_runtime.rs` | Backend itself, not a language feature — listed because per-feature JIT parity is not independently re-verified by this document; see [BACKENDS.md](BACKENDS.md) for the (separately-maintained) per-backend feature matrix. |

---

## Tier Graduation Workflows

### Experimental → Backend-Limited

When an Experimental feature has a finalized design and partial backend coverage:
1. Move from `experimental` label to `backend-limited` on related issues
2. Add feature to "Backend-Limited Features" section below with compatibility table
3. Add comprehensive tests for each supported backend
4. Document any limitations or differences between backend implementations

### Backend-Limited → Stable

When a Backend-Limited feature is implemented on all backends consistently:
1. Add to "Stable Features" section with full specification
2. Ensure test coverage across all backends (≥80%)
3. Run regression test suite across all backends
4. Remove from Backend-Limited section
5. Update this reference document

### Removing Features

When a Stable feature must be removed:
1. Document in the issue and release notes
2. Add compiler warning (not error) in the version announcing deprecation
3. Allow 3+ minor releases for users to migrate
4. Change compiler warning to hard error in next major release
5. Remove implementation code in subsequent release

---

## Relationship to the Conformance Suite (F-E1) and the Backend Matrix

This document's tiers are the direct input to two other pieces of
in-progress infrastructure (tracked under the RES-3933 roadmap, Track F):

- **Conformance suite (F-E1)**: every row in this document's Stable tier is
  the checklist for `resilient/tests/conformance/` — one file per feature,
  run via a planned `--conformance` runner mode across all three backends
  (interpreter, VM, JIT). A feature cannot be *called* Stable here without
  an entry in that inventory once F-E1 lands; until then, the "Evidence"
  column above (typechecker wiring + example corpus) is the interim
  substitute proof.
- **Backend matrix (F-E2 / [BACKENDS.md](BACKENDS.md))**: the
  Backend-Limited tier exists specifically for features whose behavior is
  not (yet) proven identical across interpreter/VM/JIT/embedded targets.
  `BACKENDS.md` maintains the per-backend, per-feature support table
  separately from this document — treat a mismatch between the two as a
  bug in whichever file is stale, not as two independent truths.
- Both efforts feed back into `STABILITY.md`: once F-E1's conformance
  suite passes a feature on every backend, that is the trigger to add the
  feature to `STABILITY.md`'s "Stable" list (closing the documentation
  gaps flagged in the classification table above) and to drop the
  "(evidence-based; not yet in `STABILITY.md`)" qualifier here.

---

## User Guidance

**Building Safety-Critical Systems:**
Use only features from the **Stable** tier. These features have the strongest compatibility guarantees and will be maintained for long-term production use.

**Building Research & Experimental Projects:**
You can use Backend-Limited and Experimental features if you understand their limitations. Check the compatibility tables and design documents (RES-* issues) before using these features.

**Library Developers:**
Document which feature tiers your library uses. If you use Experimental or Backend-Limited features, clearly state the tier and compatibility constraints in your README.

**Adopters:**
When evaluating Resilient for a project, check that your required features are in the Stable tier before committing.

---

## References

- **RES-3501**: Stabilize the language reference and feature-tier policy
- **RES-3502**: Design a real module and package system
- **RES-3503**: Unify the long-term type system roadmap
- **RES-3504**: Specify and enforce the memory model
- **RES-3505**: Consolidate the failure and recovery semantics
- **RES-3506**: Define the backend architecture contract
- **RES-3510**: Reconcile `STABILITY.md` and `docs/STABILITY_POLICY.md` into one doc — will also resolve the "evidence-based; not yet in `STABILITY.md`" rows above
- **RES-3648** (this document, RES-3501.1): create `LANGUAGE.md` with the feature-tier classification framework and populate the classification table
- **F-E1** (RES-3933 roadmap, Track F): conformance/spec suite for the stable surface — see [Relationship to the Conformance Suite](#relationship-to-the-conformance-suite-f-e1-and-the-backend-matrix) above
