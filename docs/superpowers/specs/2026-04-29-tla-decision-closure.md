# RES-396 Decision Closure — TLA+ Model Checking Integration

**Date:** 2026-04-29
**Status:** Decision summary / ticket closure for [#270](https://github.com/EricSpencer00/Resilient/issues/270)
**Tracking:** RES-396
**Companion spec:** [2026-04-26-tla-model-checking.md](2026-04-26-tla-model-checking.md)

---

## Why this document exists

The full TLA+ design is in the [2026-04-26 companion spec](2026-04-26-tla-model-checking.md).
The companion spec's "Decision log" already records maintainer
confirmation of the three primary decisions; this closure document
exists so the [#270](https://github.com/EricSpencer00/Resilient/issues/270)
ticket has a single, visible artifact a maintainer can sign off on.

[#270](https://github.com/EricSpencer00/Resilient/issues/270)'s
acceptance criteria call for six items. Three are already resolved in
the companion spec; three are administrative follow-ups that this
closure document enumerates so they can be filed as the maintainer
sees fit.

---

## Resolved (companion spec, 2026-04-26)

These three items in [#270](https://github.com/EricSpencer00/Resilient/issues/270)'s
acceptance criteria are already checked in the companion spec's
Decision log:

| Decision | Outcome | Source |
|---|---|---|
| Path A vs B vs C | **Path B** — external `.tla` + `@refines` mappings | [companion §Three integration paths](2026-04-26-tla-model-checking.md) |
| Backend default | **TLC** default; Apalache opt-in via `--mc-backend apalache` (V2.4) | [companion §TLC vs Apalache](2026-04-26-tla-model-checking.md) |
| Phasing | V2.0 (bridge) + V2.1 (`@refines`) + V2.2 (counterexample replay) ship V2; V2.3/V2.4 follow | [companion §V2.x phasing](2026-04-26-tla-model-checking.md) |

No revision needed. Cross-link is in place; this closure document
formalizes the resolution for [#270](https://github.com/EricSpencer00/Resilient/issues/270).

---

## Outstanding follow-ups

These three items are not blockers on [#270](https://github.com/EricSpencer00/Resilient/issues/270)
closing — they are forward-looking actions the maintainer (or a
delegated agent on the next sweep) should file as separate tickets.

### Follow-up 1 — V1 design-choice preservation sweep

The companion spec lists four V1 design choices that must be preserved
so V2 isn't retroactively forced into the wrong shape. These need to
be enforced in the V1 backlog — not deferred to V2.0 implementation.

| # | V1 invariant | Where to enforce | Recommended ticket |
|---|---|---|---|
| 1 | Diagnostics carry a tagged enum (extensible to `(spec_path, action_name, trace_step)`), not a flat string | [`resilient/src/diag.rs`](../../../resilient/src/diag.rs) | `RES-DIAG-TAGGED`: audit `diag.rs`, confirm extensibility, add a doc-comment marker pinning the invariant. ~1 day. |
| 2 | `live { }` block has a closed-form invariant (no arbitrary user-supplied recovery effects) | live-block parser & verifier (`recovers_to_bmc.rs`, `verifier_liveness.rs`) | `RES-LIVE-CLOSED`: add a parser check that rejects `live` blocks whose recovery body cannot be encoded as a TLA+ action. ~3 days. |
| 3 | Actor primitives ([RES-208](https://github.com/EricSpencer00/Resilient/issues/17), [RES-332](https://github.com/EricSpencer00/Resilient/issues/124), [RES-333](https://github.com/EricSpencer00/Resilient/issues/125)) define message ordering + atomicity granularity explicitly | `supervisor.rs`, `verifier_actors.rs`, SYNTAX.md | `RES-ACTOR-SEMANTICS`: open a sub-spec under `docs/superpowers/specs/` pinning down `send`/`receive` ordering and the atomicity boundary of `receive`-body. Bake outcome into [RES-332](https://github.com/EricSpencer00/Resilient/issues/124) acceptance criteria. ~1 week. |
| 4 | `recovers_to` is documented as a one-step property; multi-step is V2's `<>` | STABILITY.md, SYNTAX.md, `verifier_liveness.rs` doc-comments | `RES-RECOVERS-DOC`: one-line clarifications + a `// V2 will extend this to <>(...)` marker in the verifier. ~½ day. |

**Action:** open four GitHub issues with the labels `enhancement`,
`v1-preservation`, and a milestone tying them to V1.0 ship. Total
effort: ~2 contributor-weeks.

### Follow-up 2 — V2 open-questions resolution spec

Five open questions in the companion spec (Q1–Q5) need answers
**before** V2.0 implementation begins. They are design questions, not
implementation questions; answering them under V2.0 would force
late-stage redesign.

Recommended ticket: `RES-V2-OPEN-Qs` — a single sub-spec under
`docs/superpowers/specs/` titled "TLA+ V2.0 design lock-in" that
answers each of Q1–Q5 with a recommendation + tradeoff analysis.
Effort: ~1 contributor-week.

### Follow-up 3 — V2.0 / V2.1 / V2.2 implementation tickets

Each of the three V2 ship-phase items in the companion spec needs its
own implementation ticket. Recommended:

| Ticket | Scope | Effort |
|---|---|---|
| `RES-V2.0-bridge` | `rz tla check <file.tla>` subcommand; shells out to TLC; surface results in Resilient diagnostics | 3 weeks |
| `RES-V2.1-refines` | Parse `@refines(spec=..., action=...)`; refinement-mapping checker; `[refinement]` table in `resilient.toml`; 3 working examples + 3 failing | 4 weeks |
| `RES-V2.2-cex-replay` | Auto-generate Resilient unit test from TLC counterexample trace; depends on V2.1 mappings | 2 weeks |

These are file-on-V2-roadmap, not file-now — they should appear when
V1 ships and V2 work begins. Until then they would clutter the
backlog.

---

## Closure recommendation for [#270](https://github.com/EricSpencer00/Resilient/issues/270)

This ticket can close as soon as a maintainer:

1. Confirms the three resolved decisions above (already implicit in
   the companion spec; this closure makes it explicit).
2. Files the four V1-preservation tickets from Follow-up 1 (or
   delegates to an agent).
3. Files the V2-open-questions sub-spec ticket from Follow-up 2 (or
   defers to V2 kickoff).
4. Notes V2.0/V2.1/V2.2 from Follow-up 3 as forward-looking — to be
   filed at V2 ship time, not now.

The only actionable V1 work that gates V1.0 ship is **Follow-up 1**.
Items 2 and 3 are forward-looking and do not block V1.

---

## Cross-references

- [docs/superpowers/specs/2026-04-26-tla-model-checking.md](2026-04-26-tla-model-checking.md) — full spec
- [STABILITY.md](../../../STABILITY.md) — current V1 verification scope
- [ROADMAP.md](../../../ROADMAP.md) — V2 ladder placement (G21+)
- [#17](https://github.com/EricSpencer00/Resilient/issues/17) (RES-208), [#124](https://github.com/EricSpencer00/Resilient/issues/124) (RES-332), [#125](https://github.com/EricSpencer00/Resilient/issues/125) (RES-333) — actor design tickets that inform Follow-up 1 item 3
