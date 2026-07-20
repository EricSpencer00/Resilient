# TLA+ Actor Model — Phase B Design

**Date:** 2026-07-19
**Status:** Design decision (Phase B1) for [#3930](https://github.com/EricSpencer00/Resilient/issues/3930)
**Tracking:** RES-3930, follow-up carved out of RES-3779 (RES-3502 umbrella)
**Builds on:** [`2026-04-26-tla-model-checking.md`](superpowers/specs/2026-04-26-tla-model-checking.md),
[`2026-04-29-tla-decision-closure.md`](superpowers/specs/2026-04-29-tla-decision-closure.md),
[`2026-04-30-tla-v2-design-lock-in.md`](superpowers/specs/2026-04-30-tla-v2-design-lock-in.md)

---

## Why this document exists

#3930 scopes four pieces of work: formalize the actor/concurrency
runtime in TLA+, implement `@refines` parsing/checking (V2.1), ship a
`runtime.tla` axiom library, and resolve RES-2633 (the merged bridge —
`resilient/src/tla_bridge.rs` — is a generic `rz tla check <file.tla>`
TLC runner, not the "export contract operators to TLA+ text" feature
its original ticket title claimed).

The Q1–Q5 design-lock-in doc already answered the *shape* questions
(raw TLA+, axiomatic runtime, two-mode replay, contract-required FFI,
TLC-with-soft-timeout). What it explicitly deferred was "the exact TLA+
vocabulary in `runtime.tla`" and "the precise temporal-logic encoding"
— because at signoff time (2026-04-30), the actor runtime it was
axiomatizing over didn't exist yet. It has since landed: `actor_runtime.rs`
(mailboxes, cooperative scheduler, deadlock detection) and
`supervisor_runtime.rs` (crash events, restart policies), wired into the
interpreter's scheduler loop in `lib.rs` around line 30534, plus VM actor
support from PR #4148 (shared thread-local scheduler, deterministic
cooperative scheduling, mailboxes, deadlock detection, supervisor
crash-restart).

This document is that missing vocabulary: it maps the concrete,
shipped Rust semantics onto TLA+ spec constructs, so Phase B2
(`runtime.tla` + `Init`/`Next`/`Spec` for the scheduler) has a spec to
implement against rather than a blank page.

---

## What gets modeled vs. axiomatized

Per Q2 (axiomatic runtime, no full-system modeling), the runtime crate
stays the trusted base. But #3930's scope note explicitly asks for the
actor/concurrency *runtime model* to be formalized — not axiomatized
away. Reconciling these: **the scheduler and mailbox state machine are
modeled as a standalone TLA+ spec** (`runtime.tla` becomes the spec
itself, not just an axiom stub), because:

1. Unlike Q2's original framing (verifying arbitrary user Resilient
   programs against the runtime), Phase B's actual deliverable is
   verifying **the scheduler implementation itself** — a fixed, small,
   already-written piece of code (`actor_runtime.rs` is ~400 LOC,
   `supervisor_runtime.rs` similar). Modeling a fixed artifact once is
   cheap; modeling it per-user-program (Q2's concern) remains
   deferred to Phase B3, and stays axiomatic there.
2. This directly resolves RES-2633's naming mismatch: `runtime.tla`
   models the runtime, rather than being an axiom library that
   *assumes away* the thing #3930 asked to be formalized.
3. It gives Phase B2 a concrete, TLC-checkable artifact (small state
   space — a handful of actors, bounded mailboxes) that satisfies
   Q5's "small spec" budget without waiting on `@refines` (V2.1)
   machinery.

So the layering is:

| Layer | What it covers | Status |
|---|---|---|
| `runtime.tla` (this doc's B2 seed) | The scheduler + mailbox + supervisor state machine, modeled directly | New, Phase B2 |
| `@refines` (V2.1, Q4) | Per-user-program refinement against axioms, including FFI contract lowering | Deferred to Phase B3 |
| Axioms consumed by `@refines` | `EXTENDS runtime` — user specs import `runtime.tla`'s operators as trusted primitives once B2 lands | Phase B3 |

This is consistent with Q2: the *user-facing* verification story stays
axiomatic (a Resilient program `@refines`'s against `runtime.tla`
without re-proving the scheduler). What's new is that `runtime.tla`
itself is no longer a hand-waved axiom stub — it is checked once,
here, in Phase B2, against the real scheduler semantics below. That
one-time proof is what makes trusting it as an axiom in Phase B3
honest rather than aspirational.

---

## Mapping: Resilient actor semantics → TLA+ constructs

Grounded in `resilient/src/actor_runtime.rs`, `resilient/src/supervisor_runtime.rs`,
and the scheduler loop in `resilient/src/lib.rs` (~line 30534):

| Resilient runtime concept | TLA+ construct |
|---|---|
| `ActorPid` (u64, `fresh_pid` monotonic counter) | `Pids ⊆ Nat`, `nextPid` state variable |
| Actor registry (`register_actor` / `deregister_actor`) | `actors ⊆ Pids` state variable |
| Mailbox (`enqueue` / `dequeue`, bounded, FIFO — see `fifo_ordering_is_preserved` test) | `mailbox ∈ [Pids → Seq(Msg)]`, `Append`/`Head`/`Tail` operators, `Cardinality(mailbox[p]) ≤ MaxMailboxDepth` |
| Runnable/blocked sets (`mark_runnable`, `mark_blocked`) | `runnable, blocked ⊆ Pids` (partition of `actors`) |
| Scheduler pop order (`pop_runnable`, FIFO per `scheduler_pops_in_fifo_order` test) | `runnable` modeled as `Seq(Pids)` (not a set) so `Next` can assert FIFO pop order, not just "some enabled actor runs" |
| `is_deadlocked` (all actors blocked, none runnable) | Invariant `Deadlock ≜ actors ≠ {} ∧ runnable = ⟨⟩` — checked as a *state predicate the spec can reach*, not disallowed; deadlock-freedom is a property checked *against the program*, not the scheduler itself (a well-formed program shouldn't reach it, but the scheduler must correctly detect it when it does) |
| Single-threaded cooperative execution (only one actor runs between yield points; VM scheduler in PR #4148 is a shared thread-local, not OS threads) | `Next` is a single global action — no interleaved sub-steps within one actor's turn; this is what makes the state space small enough for TLC (Q5) — no true concurrency to interleave, only *scheduling order* nondeterminism |
| `CrashEvent` / `RestartPolicy` (Permanent / Transient / Temporary) | `Crash(pid, reason)` action; `RestartPolicy` as a TLA+ enum `{"Permanent", "Transient", "Temporary"}`; `restartCount ∈ [Pids → Nat]`, `window` tracked per Temporary-policy actor |
| Supervisor crash → restart / escalate / stop | `HandleCrash` action with policy-dispatched `CASE`, mirroring `handle_crash_event`'s branching in `supervisor_runtime.rs` |

### Nondeterminism vs. the implementation's determinism

The implementation is deterministic *given a fixed schedule* (FIFO
runnable queue, FIFO mailbox) — that's why VM/interpreter parity
matters (see the VM/interpreter backend-parity memory note) and why
the runtime doesn't need a TLA+ model to explain "what happens next"
for a *specific* run. What TLA+ adds is checking properties that hold
**across all valid interleavings of which actor sends when** — e.g.
"no send is ever silently dropped" must hold regardless of the order
distinct actors happen to call `send`. The spec therefore keeps the
scheduler's *internal* pop order deterministic (matches the Rust
implementation exactly, per the FIFO test above) while leaving
*message arrival timing from the external environment* (when a spawn
or send call happens relative to another actor's turn) as the
`Next`-level nondeterministic choice — this is the one axis where the
runtime doesn't constrain order, and it's exactly the axis TLC
explores.

---

## Invariants and temporal properties to check

| Property | TLA+ form | Rationale |
|---|---|---|
| No lost messages | `NoLostMessages ≜ □(∀ p, m : Sent(p, m) ⇒ ◇ Delivered(p, m))` | Directly tests the mailbox FIFO contract; a scheduler bug that drops a message on deregister-while-pending would violate this |
| Mailbox bound respected | `□∀ p ∈ Pids : Cardinality(mailbox[p]) ≤ MaxMailboxDepth` | Matches `enqueue_to_full_mailbox_returns_would_block`'s `WouldBlock` contract — TLC checks the *scheduler* never violates the bound it's supposed to enforce |
| Deadlock-detector soundness | `IsDetectedDeadlock ⇔ (actors ≠ {} ∧ runnable = ⟨⟩ ∧ ∀ p ∈ actors : p ∈ blocked)` | Equivalence check against the real `is_deadlocked()` logic — the property is "the TLA+ predicate and the Rust function agree on every reachable state", checked by encoding `is_deadlocked`'s exact condition and confirming TLC never finds a state where they diverge |
| Deadlock-freedom (of the scheduler itself, not user programs) | `DeadlockFreedom ≜ □(runnable = ⟨⟩ ∧ actors ≠ {} ⇒ blocked = actors)` | The scheduler shouldn't reach a state that's "stuck" without being classified as deadlocked — i.e., every non-runnable state is accounted for by the deadlock predicate, no silent hang |
| Restart convergence | `RestartConverges ≜ ∀ p : Temporary(p) ⇒ ◇□(restartCount[p] ≤ MaxRestarts)` | A `Temporary` policy actor's restart count must eventually stop growing (either it stabilizes below the limit, or the supervisor escalates) — checks `supervisor_runtime.rs`'s window-based limit logic terminates rather than restart-looping forever |
| Crash isolation | `□(Crash(p) ⇒ ∀ q ≠ p : actors[q] unaffected)` | One actor's crash doesn't corrupt another's mailbox or registry state — matches the isolation the Rust registry (per-PID `HashMap` entries) provides structurally, but worth checking explicitly since supervisor logic touches shared state |

---

## How specs get generated and checked

**Decision: hand-written `runtime.tla`, checked in CI with TLC, not
generated per-program.**

### Alternatives considered

| Option | Why not chosen |
|---|---|
| Generate a `.tla` spec per Resilient program (transpile AST → TLA+) | This is Phase B3 / V2.1 `@refines` territory and depends on machinery (contract-to-TLA+ lowering, per Q4) that doesn't exist yet. Building the generator before the hand-written reference spec exists means there's nothing to validate the generator's output against. |
| Model the scheduler as Rust-embedded property tests only (no TLA+) | Loses the entire value proposition of #3930 — property-based Rust tests (`proptest`, etc.) can't do exhaustive bounded model checking or express liveness/fairness properties like `RestartConverges`'s `◇□`. TLA+'s explicit-state exhaustive search over small bounds is precisely what's needed here and what Q5 already committed to (TLC). |
| Apalache-first (symbolic) | Q5 already locked TLC as default; Apalache is V2.4 opt-in. Revisiting this now would contradict a signed-off decision without new evidence. |
| Skip CI wiring, spec is docs-only / manually run | Fails the "checked in CI" bar that makes this maintainable — an un-run spec rots the moment `actor_runtime.rs` changes, and nothing would catch the drift. |

**Chosen approach:** `runtime.tla` is a hand-written spec, checked
into `resilient-runtime/tla/runtime.tla` (co-located with the runtime
crate it models, per Q2's original placement suggestion), with a
`.cfg` file bounding the model (small: 2 actors, mailbox depth 2 — see
Q5's benchmark-bucket precedent). CI runs `rz tla check` against it
using the existing `tla_bridge.rs` (RES-2633's actual shipped
capability), gated the same way `resilient-runtime-cortex-m-demo` gates
on its target toolchain: **skip gracefully when `tla2tools.jar` is
absent**, matching the blocker note already on #3930. This is not a
new CI mechanism — it reuses the bridge that already exists.

Rationale for hand-written over generated: the scheduler is a single,
stable, already-implemented artifact (not a moving target per
user program), so there is exactly one spec to maintain, and
hand-writing it lets the spec state variables map 1:1 to the Rust
state (`mailbox`, `runnable`, `blocked`) rather than through an
intermediate IR that would need its own correctness argument.

---

## Phased plan

### Phase B1 — this document (done)

- Design decision: model the scheduler directly in `runtime.tla`
  rather than treating it as pure axiom (reconciling #3930's
  "formalize the actor/concurrency runtime" ask with Q2's
  axiomatic-runtime default).
- Concrete mapping table (this doc) from Rust runtime state to TLA+
  constructs.
- Invariant/property list with rationale.
- Tooling decision: hand-written spec, TLC via existing
  `tla_bridge.rs`, jar-gated CI.

**Acceptance criteria:** this document merged; unblocks B2 by giving
it a spec to write instead of a design question to answer.

### Phase B2 — `runtime.tla` scheduler spec + gated CI check

- Write `resilient-runtime/tla/runtime.tla`: `Init`, `Next` (single
  global action per the "cooperative, not interleaved" note above),
  `Spec`, and the five properties from the table above.
- Write `resilient-runtime/tla/runtime.cfg` with the small-bound model
  (2–3 actors, mailbox depth 2) per Q5's size-bucket precedent.
- Add a CI job (or extend an existing one) that runs
  `rz tla check resilient-runtime/tla/runtime.tla` — skips with an
  informational note when `tla2tools.jar` is absent (matches
  `RESILIENT_TLC_JAR` discovery already in `tla_bridge.rs`); this
  mirrors `tla-perf-gate`'s informational-not-required posture from
  Q5 so an absent jar never blocks merge.
- Unit-test-level cross-check: assert the TLA+ `Deadlock` predicate's
  Boolean structure matches `is_deadlocked()`'s Rust condition
  line-for-line in a code comment (manual audit, not automated —
  automating the equivalence check is out of scope for B2).

**Acceptance criteria:** `runtime.tla` exists, TLC finds no
violations of the five properties within the 5-minute budget (Q5) on
the bounded model, CI job wired and jar-gated, doesn't block merge
when jar is absent.

### Phase B3 — `@refines` (V2.1) + per-program extraction, if warranted

- Implement `@refines(spec=..., action=...)` parsing (currently
  entirely absent from `resilient/src/*.rs` — confirmed via grep).
- Checker walks the annotated function's reachable call graph,
  rejecting `extern fn` calls without `requires`/`ensures` (Q4).
- `EXTENDS runtime` becomes valid once B2's `runtime.tla` exists to
  extend.
- Re-evaluate "if warranted": if usage data from B2 (do maintainers
  actually reach for `@refines` on real actor code) doesn't justify
  the ~4-contributor-week estimate from the decision-closure doc,
  scope B3 down to documentation-only ("write your `@refines` target
  by hand against `runtime.tla`, no compiler-side annotation checker
  yet") and re-ticket the full checker separately.

**Acceptance criteria:** tracked in a follow-up ticket filed at the
start of B3, per the decision-closure doc's "file at V2 ship time, not
now" guidance — filing now would clutter the backlog ahead of B2's
learnings.

---

## What this document does NOT decide

- The literal TLA+ syntax inside `runtime.tla` (left to B2 — this doc
  gives the vocabulary, not the file).
- Whether B3's `@refines` checker ships as part of the typechecker
  pass or a standalone `rz tla refines-check` subcommand — that's a
  B3-time call once the checker's actual complexity is known.
- Any change to the V1 actor language surface (`spawn`/`send`/`receive`
  syntax, contracts) — this is a verification-tooling document, not a
  language-semantics change.
- Counterexample replay UX (Q3 already settled this at the V2.0/V2.2
  level; B2's spec just needs to emit the `@kind` action labels Q3
  requires so B3/V2.2 replay can bucket them correctly).

---

## Cross-references

- [#3930](https://github.com/EricSpencer00/Resilient/issues/3930) — this ticket
- [#3779](https://github.com/EricSpencer00/Resilient/issues/3779) (RES-3502 umbrella, closed after Phase A/C)
- RES-2633 — bridge naming mismatch (no longer has an open tracking issue; noted in #3930's scope) resolved by this doc's B2/B3 split
- `resilient/src/actor_runtime.rs`, `resilient/src/supervisor_runtime.rs` — the Rust semantics this spec models
- `resilient/src/tla_bridge.rs` — existing `rz tla check` bridge, reused (not replaced) by B2's CI wiring
- `docs/superpowers/specs/2026-04-30-tla-v2-design-lock-in.md` — Q1–Q5 decisions this doc builds on
- `docs/FAILURE_MODEL.md` — runtime failure taxonomy referenced by `CrashReason`
