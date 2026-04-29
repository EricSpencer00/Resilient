# TLA+ Model Checking Integration — V2+ Spec

**Date:** 2026-04-26
**Status:** Spec / Future Work
**Scope:** Add temporal-logic model checking to Resilient. Z3-based contract
verification stays the V1 surface; this is the V2+ ladder above it.
**Tracking:** RES-396 (#270)

---

## TL;DR

Resilient's V1 verifier is **state-local**: Z3 checks `requires` / `ensures`
clauses on each function, plus the cluster-level static invariants RES-318
and the `recovers_to` postcondition. It cannot reason about
**traces** — sequences of states a system passes through over time — which
is where most safety-critical bugs live. TLA+ is the canonical industrial
tool for trace reasoning. This spec describes how to graft TLA+
capabilities onto Resilient without replacing the V1 surface.

The V1 design choice this protects: **don't bake state-local Z3 assumptions
into the AST or the diagnostic format** in a way that makes adding a trace
layer impossible later.

---

## What Z3 already gives us

| Property | Where Z3 proves it |
|---|---|
| `requires` / `ensures` arithmetic | `verifier_z3.rs` — NIA + LIA |
| Bitwise / shift correctness | `verifier_z3.rs` — BV theory (RES-354) |
| Array bounds (proven path) | `bounds_check.rs` + Z3 fallback |
| Linear-type non-double-use | `linear.rs` |
| Region aliasing (structural) | `verifier_actors.rs::cluster_*` |
| `recovers_to` postcondition (single transition) | `verifier_liveness.rs` |
| Loop invariant preservation (single iter) | `verifier_loop_invariants.rs` |

Each of these is **single-state** (one program point, one set of variable
bindings) or **single-transition** (one function call, one loop step). The
verifier never reasons about an unbounded sequence of states.

---

## What Z3 cannot give us

These are the bug classes Resilient cannot statically rule out today, even
with `--features z3`:

1. **Liveness over time.** "If a sensor stops responding for 50 ms, the
   watchdog resets the bus before the next sample is read." The watchdog
   reset is a temporal property over a run, not a per-call ensures.
2. **Multi-actor interleavings.** RES-318 cluster invariants check
   structural properties of an actor system at a snapshot. They cannot
   express "no two replicas claim leadership in the same epoch across all
   message orderings."
3. **Refinement.** "The compiled bytecode is observationally equivalent to
   the source semantics." We have differential testing (#101) but not a
   formal refinement statement.
4. **Stuttering / progress.** "Eventually the queue is drained" is a
   liveness property; Z3 only proves safety properties (nothing bad ever
   happens), not progress (something good eventually happens).
5. **Fairness assumptions.** "Under weak fairness on the IO scheduler, the
   live block always retries after a transient fault" — needs an explicit
   fairness model.

These are TLA+'s bread and butter. They map to TLA+'s `[]`/`<>`/`~>`
operators, the `WF`/`SF` fairness combinators, and refinement via
`Spec1 => Spec2` with a refinement mapping.

---

## Three integration paths (pick exactly one for V2)

### Path A — Embedded sub-language (`spec { … }` blocks)

Add a `spec` keyword to the surface language. Inside `spec { }`, the user
writes a tiny temporal DSL that compiles to TLA+:

```rz
spec leader_election {
    state: epoch: Int, leader: Option<NodeId>;

    init { epoch == 0 && leader == None }

    action elect(n: NodeId) {
        when leader == None;
        next epoch == epoch + 1 && leader == Some(n)
    }

    invariant single_leader: forall a, b in nodes . leader == Some(a)
                              && leader == Some(b) => a == b;
    liveness   eventual_leader: <> (leader != None);
}
```

The compiler emits a `.tla` file behind the scenes, calls TLC, and surfaces
results as Resilient diagnostics. The user never opens TLA+ directly.

**Pros**
- Single source of truth — the spec lives next to the implementation.
- Refinement mapping is implicit: state variables in `spec` blocks must
  map to actual program state, enforced at compile time.
- Discoverable — users learn temporal reasoning incrementally without
  having to context-switch into TLA+ syntax.

**Cons**
- Big lexer/parser/typechecker surface to add and stabilize.
- Limits expressiveness to what the DSL covers; escape hatch needed for
  power users.
- Tooling (highlighting, LSP, formatter) must learn the spec sub-language.

### Path B — External `.tla` files with refinement mappings

Users write standard TLA+ in `.tla` files alongside their `.rz` source.
Resilient ships a `refinement` annotation that ties Resilient functions /
state to TLA+ actions / variables:

```rz
@refines(spec = "election.tla", action = "Elect")
fn elect_leader(epoch: Int, candidate: NodeId) -> Bool { … }
```

The compiler runs TLC on the spec, then checks the refinement mapping at
compile time (i.e., the Resilient implementation actually corresponds to
the action it claims to refine).

**Pros**
- Reuse the entire TLA+ ecosystem — TLC, Apalache, the Toolbox, all of
  Lamport's pedagogy.
- No new sub-language to maintain.
- Refinement is explicit and reviewable.

**Cons**
- Two source-of-truth files (the `.rz` and the `.tla`), with a refinement
  bridge that has to stay in sync. This is exactly the gap that bites every
  "spec next to code" approach historically.
- Users have to learn TLA+ syntax — high barrier.
- The refinement-mapping check is itself nontrivial to mechanize.

### Path C — Annotation-driven spec extraction

Users add `@invariant`, `@always`, `@eventually`, `@stable`, `@fair`
annotations to existing Resilient code. The compiler **synthesizes** a TLA+
spec from the annotated code (treating function bodies as actions) and
runs TLC on the synthesis.

```rz
@always(epoch >= old_epoch)
fn step_epoch(epoch: Int) -> Int { return epoch + 1; }

@eventually(queue.is_empty())
@fair  // assume scheduler eventually picks every actor
actor Drainer { … }
```

**Pros**
- Lowest cognitive barrier — users annotate what they know, the synthesis
  does the rest.
- Spec stays embedded in the implementation; no separate file.
- Plays well with `live` blocks and the actor scaffolding the language
  already has.

**Cons**
- Synthesis is hard to make sound. Translating loops, mutable state, and
  call graphs into TLA+ actions has subtle pitfalls (atomicity granularity,
  variable scoping, nondeterminism injection).
- Hard to fall back gracefully when synthesis can't translate a feature
  (e.g. closures, FFI calls, recursive higher-order patterns).
- Encourages the smell of "trust me, the synthesis is right." Reviewers
  can't audit a spec they can't see.

### Recommendation

**Path B** for V2 (external `.tla` + refinement mapping). Rationale:
- Lowest implementation cost — most of the work is the refinement-mapping
  checker, not a new sub-language.
- Honest — users see the actual TLA+ they're committing to. No magic.
- Composable — Path A and Path C can be added in V3+ on top of Path B
  (the embedded DSL and the synthesis are both compilers TO TLA+; if Path
  B's refinement-mapping checker exists, the higher-level paths reuse it).

**Counter-argument considered:** Path A is the "Resilient experience" the
project's positioning would suggest (one source of truth, learn-as-you-go).
But the V1 verifier is already a custom DSL that users have to learn; piling
a second one on for temporal properties multiplies the learning surface.
TLA+ has 25 years of pedagogy. Use it.

---

## TLC vs Apalache (for V2's model-checker backend)

| Property | TLC | Apalache |
|---|---|---|
| Maturity | Production since ~1999 | ~2018, active research |
| State space | Explicit-state — enumerates concrete states | Symbolic — uses an SMT solver (Z3) under the hood |
| Scalability | Good up to ~10⁹ states; bad with unbounded data | Bounded model checking; good for small-but-deep specs |
| Counterexamples | Concrete trace, easy to read | Concrete trace via Z3 model |
| TLA+ compatibility | The reference implementation | Subset (PlusCal partially supported) |
| License | MIT | Apache-2.0 |
| Distribution | Single jar (~30 MB), JRE required | Native (Scala / GraalVM); ~80 MB |

**Recommendation: ship TLC as the default, leave Apalache as a
configuration option.** Reasoning:

- TLC is the de-facto standard, every TLA+ tutorial assumes it.
- TLC's enumerative explosion is fine for the size of system Resilient
  users will model in V2 (hundreds of states, not billions).
- Apalache shines on bounded depth with rich data — a niche we'll learn
  whether we need from real V2 user feedback.
- Both consume the same `.tla` file, so swapping later is a
  configuration change, not a rewrite.

Embedding strategy: shell out to the JVM (`java -jar tla2tools.jar`).
Resilient's CI already has Java for other tooling; users get a one-line
install instruction (or we ship a `resilient install-tla` helper that grabs
the jar). **Do NOT** vendor TLC into the binary — license keeps it
permissible but the binary size hit isn't worth it.

---

## V2.x phasing

Each phase is a separate ticket. Estimates are wall-clock for one focused
contributor; halve them if work parallelizes.

### V2.0 — bridge (≈3 weeks)

- `rz tla check <file.tla>` subcommand: shells out to TLC, parses output,
  surfaces results in Resilient's diagnostic format (`path:line:col`,
  source caret, error class).
- `cargo install --features tla` opt-in feature so the dep tree stays light
  for the default build.
- One worked example in `docs/tla/` (a counter, a queue, a leader election)
  with TLC output verified in CI.
- **No refinement yet.** The spec and the implementation are two unrelated
  things; this phase only proves the spec is consistent.

### V2.1 — `@refines` annotation (≈4 weeks)

- Parser support for `@refines(spec = "X.tla", action = "Y")`.
- Refinement-mapping checker: at compile time, verify the annotated
  function's pre/post pair maps onto the named TLA+ action. The
  per-variable mapping is read from a `[refinement]` section in
  `resilient.toml`.
- Diagnostic: `error[refinement]: function elect_leader doesn't refine
  action Elect — invariant epoch_monotonic violated on path …`.
- Tests: at least three working examples of refinement (counter, queue,
  leader election) plus three deliberate-failure examples.

### V2.2 — counterexample replay (≈2 weeks)

- When TLC finds a counterexample, automatically generate a Resilient unit
  test that reproduces it. The test sets up the initial state from the TLC
  trace, runs the implementation steps, and asserts the violated property.
- This closes the "TLC says my spec is broken; what do I do?" loop. Users
  can run the failing trace under `rz`, debug, fix, re-run TLC.
- Depends on V2.1 (need the refinement mapping to know which Resilient
  functions correspond to which TLC steps).

### V2.3 — fairness + liveness (≈3 weeks)

- Surface the WF/SF fairness operators in the diagnostics layer.
- Default the embedded `live { }` block to a weak-fairness assumption
  (matches the retry semantics already documented). Power users can
  override.
- Worked example: prove `<>(queue.is_empty())` for a drainer actor under
  weak fairness on the message scheduler.

### V2.4 — Apalache backend (≈2 weeks, optional)

- `--mc-backend apalache` flag.
- Configuration matrix for Apalache's bounded depth and SMT timeout.
- Side-by-side comparison docs in `docs/tla/backends.md`.

**V2 ships when V2.0 + V2.1 + V2.2 are all green.** V2.3 and V2.4 are V2.x
follow-ons.

---

## V1 design choices this spec asks us to preserve

These are decisions the V1 work should NOT make in a way that closes
V2 doors:

1. **Diagnostic format must stay extensible.** Today's diagnostics carry
   `(path, line, col, code, message)`. V2's TLA+ diagnostics will need
   `(spec_path, action_name, trace_step)` too. Diagnostics should already
   be a tagged enum, not a flat string — verify this in `diag.rs`.
2. **`live { }` semantics must be expressible as a TLA+ action.** The V1
   `live` block has a retry budget and a state-restoration step; both have
   clean TLA+ encodings. Don't add `live`-block features that can't be
   modeled (e.g. arbitrary user-supplied recovery effects with no closed
   invariant).
3. **Actor primitives (RES-208, RES-332, RES-333) must define their
   message ordering and atomicity granularity explicitly.** V2's TLC
   encoding will need to know whether `send` is FIFO per pair, whether
   `receive` is atomic with the body that follows, etc. If V1 ships actors
   with under-specified semantics, V2 will have to make the choice
   retroactively — and pick the wrong one for half the existing programs.
4. **`recovers_to` postcondition is a one-step property today.** Document
   that it's intentionally one-step; the multi-step / liveness version is
   V2's `<>` operator. Don't over-promise on `recovers_to`.

---

## Open questions

These need answers before V2.0 lands:

- **Q1.** Which TLA+ idioms do we want to expose, and which do we want to
  hide behind macros / conveniences? (e.g. the `Init`/`Next`/`Spec` boilerplate
  is repetitive; a Resilient-side template could generate it.)
- **Q2.** Do we model the runtime crate (`resilient-runtime`) as part of
  the system being verified, or treat it as an axiomatic library? The
  former is more honest but expensive; the latter requires writing TLA+
  axioms for every runtime primitive.
- **Q3.** Counterexample fidelity — when TLC reports a 12-step trace, does
  the Resilient-side replay show 12 user-meaningful steps, or do internal
  steps of the actor scheduler clutter the output?
- **Q4.** How does this interact with the FFI? Foreign function calls are
  observably nondeterministic; do we treat them as `CHOOSE x \in T : TRUE`
  or require a contract?
- **Q5.** Performance — TLC is enumerative. What's our expected
  state-space size for a "reasonable" Resilient program, and at what point
  do we tell users to switch to Apalache? An early-V2 bench under
  `benchmarks/tla/` would answer this.

These are exactly the kinds of design questions you don't want to
discover during V2 implementation. Pre-V2, write a follow-up spec that
answers all five.

---

## What this spec does NOT commit to

- Replacing Z3. Z3 stays the V1 surface for state-local properties; TLA+
  is purely additive.
- A "verified" ribbon on the language. We're shipping a *tool*, not a
  certification claim. The verification limitations doc (RES-DOCS, V1
  deliverable) should be updated to mention TLA+ as a future capability,
  not a current one.
- A custom model checker. We're shelling out to TLC / Apalache. Building
  our own would be a multi-year effort and would absorb the project.

---

## Cost estimate

| Phase | Eng-weeks (one contributor) | New deps | New surface |
|---|---:|---|---|
| V2.0 bridge | 3 | TLC jar (external) | 1 subcommand |
| V2.1 refinement | 4 | none | 1 annotation |
| V2.2 cex replay | 2 | none | 1 test generator |
| V2.3 fairness | 3 | none | extends `live` |
| V2.4 Apalache | 2 | Apalache jar (external) | 1 flag |
| **V2.0–2.2 (ship V2)** | **9** | TLC | 1 subcommand + 1 annotation + 1 test gen |

Plus ~1 week of doc work (tutorial + reference + verification-limitations
update) per phase that ships.

Total V2 effort: **~10 contributor-weeks**, single-person. Spread across
the project's normal cadence, that's a 3–4 month tail after V1.0.

---

## Decision log

Maintainer review 2026-04-26 — defaults confirmed:

- [x] **Path B** (external `.tla` files + `@refines` mappings). Reuses the
      TLA+ ecosystem, lowest implementation cost, honest about what's
      being verified. Path A and Path C remain candidates for V3+ on top
      of Path B's refinement-mapping checker.
- [x] **TLC** as the default backend. Apalache stays available behind
      `--mc-backend apalache` (V2.4). Both consume the same `.tla` so
      switching is a config change.
- [x] **V2 ship scope = V2.0 + V2.1 + V2.2.** Fairness/liveness (V2.3) and
      Apalache (V2.4) are V2.x follow-ons, not gates on the V2 ship.
- [ ] **V1 design choices preservation** — open follow-up: explicit sweep
      across the V1 actor backlog (RES-208, RES-332, RES-333) to confirm
      message-ordering and atomicity granularity are pinned down before
      V1 ships. Tracked under RES-396 follow-ons in
      [2026-04-29-tla-decision-closure.md](2026-04-29-tla-decision-closure.md)
      (Follow-up 1).

---

## Cross-references

- [STABILITY.md](../../../STABILITY.md) — current verification scope claim
- [docs/superpowers/specs/2026-04-20-ffi-v2-design.md](2026-04-20-ffi-v2-design.md)
  — the analogous "extend an existing surface across all backends" pattern
- ROADMAP.md — V2 ladder; currently empty after G20 (self-hosting), this
  spec proposes G21+ as the TLA+ ladder
- Lamport's "Specifying Systems" (2003) — the canonical TLA+ reference;
  required reading before V2.0 work begins
- Apalache docs: <https://apalache.informal.systems/>
- TLC docs: <https://lamport.azurewebsites.net/tla/tla.html>
