---
title: v1.x Roadmap
parent: Language Reference
nav_order: 10
permalink: /roadmap-v1x
---

# Resilient v1.x Roadmap

> **Parent tracker:** [#4117](https://github.com/EricSpencer00/Resilient/issues/4117).
> This document picks up where [`docs/ROADMAP_PHASE2.md`](ROADMAP_PHASE2.md) and the
> [v1.0 tracker `#3933`](https://github.com/EricSpencer00/Resilient/issues/3933) (closed
> 2026-07-16) left off. `v1.0.0` shipped on commit `1f66c63c`; everything below is
> post-1.0 scope, organized into seven tracks and grouped into rough `v1.1`–`v1.4`
> milestones. Milestone numbers are sequencing hints, not calendar commitments — any
> ticket can be picked up out of order per `CLAUDE.md`'s ship-to-merge model.

## How to use this doc

- Each track lists open issues (existing + newly filed) with a one-line scope note.
- "Milestone" groups tracks into a rough v1.1/v1.2/v1.3/v1.4 sequence based on
  dependency order (type system before verification depth that consumes it; VM
  parity before embedded v2 which builds on the same VM).
- New tickets filed alongside this doc are marked **(new)**.
- The full checklist lives in tracker issue **#4117** — keep that issue's checkboxes
  in sync with issue state; keep the narrative and sequencing here.

---

## Track 1 — Type system completion (Milestone: v1.1–v1.2)

Finishes the deep `typechecker.rs`/`lib.rs` work sequenced in `ROADMAP_PHASE2.md`
Track A. Serialize within this track — it's one core file.

| Issue | Scope |
|---|---|
| [#4095](https://github.com/EricSpencer00/Resilient/issues/4095) | `dyn Trait` v2: vtable codegen + object-safety + generic position (type-checking side already shipped, #4096) |
| [#4097](https://github.com/EricSpencer00/Resilient/issues/4097) | A-E7: named effect-variable unification across multiple HOF params + lambda/local-var callback resolution |
| [#4079](https://github.com/EricSpencer00/Resilient/issues/4079) | A-E5: Copy-vs-Move design decision — blocks use-after-move checking on unannotated bindings |
| [#4070](https://github.com/EricSpencer00/Resilient/issues/4070) | A-E5: use-after-move + conditional-path region inference for unannotated code |
| [#4067](https://github.com/EricSpencer00/Resilient/issues/4067) | A-E3: associated-type projections beyond `Self`-in-return-position |
| [#4109](https://github.com/EricSpencer00/Resilient/issues/4109) **(new)** | A-E2: compound trait bounds (`T: A + B`) + minimal const-generics (`array<T, N>`) |
| [#4110](https://github.com/EricSpencer00/Resilient/issues/4110) **(new)** | A-E6: module system completeness — `pub use` re-export, glob-import, circular-import diagnostics |
| [#3977](https://github.com/EricSpencer00/Resilient/issues/3977) | Extend `arr[i]` element tracking to method returns, nested `Option<array<T>>`, const-generic lengths (depends on #4109's const-generics landing) |

**Sequencing note:** #4079's Copy/Move decision blocks #4070. #4109's const-generics
piece should land before #3977 tries to track const-generic array lengths.

**Explicitly out of scope for v1.x (see deferral list below):** Phase 2 (effects
annotation syntax `! IO`) and Phase 3 (bidirectional/local type inference) from
`docs/TYPE_SYSTEM_ROADMAP.md` remain design-stage; no ticket exists yet because the
grammar work is a prerequisite design decision, not a shippable increment. Revisit
once Track 1 above is closed.

---

## Track 2 — VM/backend parity (Milestone: v1.1)

Continuation of `ROADMAP_PHASE2.md` Track B. The silent-wrongness gate is closed;
remaining items are loud `Unsupported` feature-completeness gaps.

| Issue | Scope |
|---|---|
| [#4060](https://github.com/EricSpencer00/Resilient/issues/4060) | VM lowering for `Node::Quantifier` (forall/exists) and `Node::DeferStatement` |
| [#4063](https://github.com/EricSpencer00/Resilient/issues/4063) | VM completeness tail: actor execution, call-stack introspection, nested-fn closure capture |
| [#4111](https://github.com/EricSpencer00/Resilient/issues/4111) **(new)** | B-E4: JIT completeness — string/struct lowering, differential test pass, startup benchmarks |
| [#4108](https://github.com/EricSpencer00/Resilient/issues/4108) | perf-gate: `jit_tail_rec` micro-benchmark flake (infra hygiene, not feature work) |

---

## Track 3 — Embedded runtime v2 (Milestone: v1.2–v1.3)

Builds on Track 2's VM work. `fn`-call support landed in v1.0 (D-E1 #4082); this
track is the remaining tail plus the portability enforcement Track D-E3 called for.

| Issue | Scope |
|---|---|
| [#4083](https://github.com/EricSpencer00/Resilient/issues/4083) | Embedded fn-support v2 tail: closures, fails, postchecks, embedded fn smoke test |
| [#4116](https://github.com/EricSpencer00/Resilient/issues/4116) **(new)** | D-E3: stdlib portability lint — reject/graceful-stub Tier-2/3 builtins on `no_std`/`wasm32` targets |

**Deliberately deferred, not ticketed yet:** array/heap types on-device behind an
`alloc` gate (22/54 opcodes need `alloc`) and `#[interrupt(...)]` end-to-end lowering
under QEMU. Both are large, `needs-design` scope; file as follow-ups once #4083 lands
and the embedded call-frame model stabilizes — filing them now would duplicate design
work #4083 is likely to reshape.

---

## Track 4 — Formal verification depth (Milestone: v1.2–v1.3)

Continuation of `ROADMAP_PHASE2.md` Track C, the project's core differentiator.

| Issue | Scope |
|---|---|
| [#4112](https://github.com/EricSpencer00/Resilient/issues/4112) **(new)** | C-E3: wire `prove_overflow_safe` (BV64) into the `requires`/`ensures` static verification path |
| [#3930](https://github.com/EricSpencer00/Resilient/issues/3930) | TLA+ Phase B: actor/concurrency model checking (tooling-blocked on vendoring `tla2tools.jar` in CI) |
| [#3859](https://github.com/EricSpencer00/Resilient/issues/3859) | Tier 3 contract proof certificates (JSON audit artifact) |

---

## Track 5 — Release engineering (Milestone: v1.1)

| Issue | Scope |
|---|---|
| [#4113](https://github.com/EricSpencer00/Resilient/issues/4113) **(new)** | Static Z3 linking on the remaining `x86_64-apple-darwin` release target (3/4 done per #4101) |

---

## Track 6 — MCP server productionization (Milestone: independent track, any time)

This is explicitly a separate product from the language per the #3933 closing
comment, not v1.0/v1.x language scope, but it's an active umbrella worth keeping
visible in the v1.x tracker since agents pick up tickets from the same issue pool.

| Issue | Scope |
|---|---|
| [#3934](https://github.com/EricSpencer00/Resilient/issues/3934) | Live MCP Server — Phase 2+ (hardening, deploy, monitoring) umbrella, children #3935–#3968 |

No new tickets filed here — the umbrella already has ~34 open children covering
auth, rate limiting, deploy targets, observability, and client examples in detail.

---

## Track 7 — Tooling & DX (Milestone: v1.3–v1.4)

Continuation of `ROADMAP_PHASE2.md` Track E, minus vsce (maintainer-only, `E-E3`,
already resolved for v1.0 — see `reference_vsce_pat` memory, don't touch without
explicit instruction).

| Issue | Scope |
|---|---|
| [#4114](https://github.com/EricSpencer00/Resilient/issues/4114) **(new)** | E-E2: package-manager registry protocol — `rz pkg update`, static JSON index, checksum verification |
| [#4115](https://github.com/EricSpencer00/Resilient/issues/4115) **(new)** | E-E4: diagnostic error-code (`E####`) convention + `rz explain` + generated `docs/errors/*.md` |

---

## Deferral-list disposition (from the v1.0-rc readiness memory)

The following items were explicitly deferred to 1.x at v1.0 ship time. Disposition
of each:

| Deferred item | Disposition |
|---|---|
| `dyn Trait` vtable codegen/object-safety | Ticketed — Track 1, #4095 |
| Effect-variable unification/lambda callbacks | Ticketed — Track 1, #4097 |
| Use-after-move region checking | Ticketed — Track 1, #4079/#4070 |
| VM completeness tail (quantifiers/defer, actors/introspection/closures) | Ticketed — Track 2, #4060/#4063 |
| Embedded fn v2 tail + `#[interrupt]` e2e | Ticketed — Track 3, #4083; interrupt e2e deliberately not yet re-ticketed (see Track 3 note above) |
| Module glob-import | Ticketed — Track 1, #4110 (design decision on whether to implement or re-affirm the cut is folded into that ticket) |
| Z3 on `x86_64-apple-darwin`; static Z3 macOS/aarch64 secondary polish | Ticketed — Track 5, #4113 (aarch64 already resolved by #4101, only the x86_64 leg remains) |
| MCP-server umbrella #3934 (#3935–3968) | Tracked visibly (Track 6) but intentionally out of v1.x language scope — separate product |
| Codex design lock-ins #3501–3509 | Already closed at v1.0 — no action |
| TLA+ Phase B #3930 | Ticketed — Track 4, already open, referenced not duplicated |

**Skipped entirely (not re-ticketed), with reasoning:**
- **Effect annotation syntax / Phase 2+3 of `docs/TYPE_SYSTEM_ROADMAP.md`** (the `! IO`
  grammar, bidirectional inference) — no ticket filed. This needs a maintainer-level
  grammar design decision before it decomposes into shippable increments; forcing a
  ticket now would just be a restatement of the existing roadmap doc's "Design stage"
  status. Revisit once Track 1's effect-variable work (#4097) lands, since that's the
  closest existing code to build on.
- **Array/heap types on-device + `#[interrupt]` e2e** — see Track 3 note; deferred
  pending #4083's shape, not skipped outright.

---

## Milestone summary

| Milestone | Tracks in scope |
|---|---|
| v1.1 | Track 2 (VM/backend parity), Track 5 (release: Z3 on last macOS target), start of Track 1 |
| v1.2 | Track 1 continues, Track 3 begins (embedded v2), Track 4 begins (verification depth) |
| v1.3 | Track 3/4 continue, Track 7 begins (tooling/DX) |
| v1.4 | Track 7 continues, effect-annotation grammar design revisited, any residual Track 1 items |
| Ongoing | Track 6 (MCP server) — independent cadence, not gated on language milestones |

---

## References

- [`docs/ROADMAP_PHASE2.md`](ROADMAP_PHASE2.md) — the v1.0 roadmap this document
  supersedes for planning purposes (left in place as historical record).
- [`docs/TYPE_SYSTEM_ROADMAP.md`](TYPE_SYSTEM_ROADMAP.md) — Phase 2/3 effects and
  inference design, still design-stage.
- [`docs/RELEASE_AUDIT.md`](RELEASE_AUDIT.md) — release target matrix referenced by
  Track 5.
- [`docs/STABILITY_POLICY.md`](STABILITY_POLICY.md) — SemVer commitment on the Stable
  surface as of v1.0.0.
- Tracker issue [#4117](https://github.com/EricSpencer00/Resilient/issues/4117).
