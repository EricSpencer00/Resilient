# Region System — Implementation Plan

**Date:** 2026-04-30
**Status:** Design lock-in for the region-work chain
**Tracking:** [#219 RES-393](https://github.com/EricSpencer00/Resilient/issues/219), [#220 RES-394](https://github.com/EricSpencer00/Resilient/issues/220), [#221 RES-395](https://github.com/EricSpencer00/Resilient/issues/221)
**Existing infra:** RES-391 (parsed reference-type annotations + syntactic non-aliasing check), RES-392 (Z3 fallback that lifts the borrow-check cliff for cases where the syntactic rule is conservative)

---

## Context

The three open region tickets form a chain:

- **#219 RES-393**: Z3-backed alias analysis for ownership regions — replaces the syntactic non-aliasing check with a real solver call when the syntactic rule rejects a benign aliasing.
- **#220 RES-394**: Region inference for unlabeled references — `&T` without `[label]` should auto-infer a region from the call graph; today the user has to spell out every label.
- **#221 RES-395**: Region polymorphism on functions and types — `fn foo<R>(...)` should let R be a region parameter that the caller fills in.

They're chained because each tightens the system in a way that makes the next one productive: Z3-backed analysis is what region inference uses to discharge non-aliasing obligations between inferred regions; region polymorphism is the user-facing feature that lets a function generic over regions interact with whatever region the caller is in.

The existing RES-391 / RES-392 infrastructure shipped the parsed surface (`& T`, `&mut T`, `&[A] T`, `&mut[A] T`) and the syntactic check (`crate::check_region_aliasing`) — that's the runway each subsequent PR builds from.

## Recommended decomposition

### Path A — ship #219 first

#219 tightens what the existing `check_region_aliasing` accepts. PR 1: add the Z3 fallback when the syntactic rule rejects (already partially scaffolded under the `feature = "z3"` flag — extend it to actually emit and discharge an alias-set obligation). PR 2: golden tests that exercise the fallback on cases the syntactic rule can't decide.

After #219 lands, agents can start writing programs whose aliasing depends on caller-side preconditions.

### Path B — #220 region inference

Once #219 has the alias-checking infrastructure, region inference is the natural follow-up. The user writes `&T` (no region label); the inference pass walks the call graph and assigns a region. Conflicts (two inferred regions that the alias analysis can't separate) become typed errors.

PR 1: a fresh-region generator + a unification table mapping `(call site, parameter)` to region. PR 2: the inference pass itself, running after typechecking. PR 3: integration with the existing `check_region_aliasing` — inferred regions feed in alongside user-labeled ones.

### Path C — #221 region polymorphism

Once inference works, polymorphism is the user-facing payoff: `fn foo<R>(&[R] data) -> &[R] T { … }` lets the function be reused across multiple call sites with different regions. PR 1: parser + AST extension for `<R>` region parameters. PR 2: typechecker substitution at call sites. PR 3: integration with #220's inference (the inferred regions become candidates for the substitution).

## Total estimated effort

- #219: 2–3 PRs, ~1 week
- #220: 3 PRs, ~1.5 weeks
- #221: 3 PRs, ~1.5 weeks

Combined: roughly 4 weeks of focused work for a single contributor, longer if interleaved.

## Out of scope

- Full lifetime polymorphism in the Rust sense (regions in Resilient are simpler — they're equivalence classes of references that must not alias mutably, not lifetime tokens with subsumption rules). The corresponding ticket would be RES-NNN-future, far beyond the chain above.
- Region-tagged heap allocation (RES has no general heap today; allocations are bounded to the runtime's `no_std` arenas).
- Cross-thread region annotations — not relevant until concurrency expands beyond the cooperative scheduler in [#124 RES-332](https://github.com/EricSpencer00/Resilient/issues/124).

## Why this is doc-only today

Each ticket needs careful surgical work in `crate::region_check` (~1k LOC today) and `crate::verifier_z3` (the Z3 emitter). Bundling them into a single PR risks losing the per-pass review window that the auto-merge flow depends on. The recommendation: ship them as the three sequential workstreams above, each landing on its own merge cycle, with the design-lock-in this doc captures fixing the interfaces between them ahead of time.
