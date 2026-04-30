# TLA+ V2.0 Design Lock-in

**Date:** 2026-04-30
**Status:** Design recommendations / decision lock-in for #373
**Tracking:** RES-V2-OPEN-Qs (issue #373)
**Companion:** [2026-04-26-tla-model-checking.md](2026-04-26-tla-model-checking.md), [2026-04-29-tla-decision-closure.md](2026-04-29-tla-decision-closure.md)
**Unblocks:** V2.0 implementation, V2.1 `@refines`, V2.2 counterexample replay; the actor-semantics ticket [#361 RES-ACTOR-SEMANTICS](https://github.com/EricSpencer00/Resilient/issues/361) consumes Q4's recommendation directly

---

## Why this document exists

The companion 2026-04-26 spec lists five open questions
("Q1–Q5") that must be answered **before** V2.0 implementation
begins. Discovering the answers during implementation forces
late-stage redesign — the cost of a wrong call on these
questions ranges from hours of rework (Q1, Q3) to weeks of
re-modeling (Q2, Q4).

The companion spec authors deliberately separated this
sub-spec from the parent so the recommendations can be
maintainer-signed-off independently of the broader phasing
decision (which the
[2026-04-29-tla-decision-closure](2026-04-29-tla-decision-closure.md)
already locked in).

This document gives each Q a recommendation, the tradeoff
analysis behind it, and a "fold back into V2.0" line that names
the exact ticket whose acceptance criteria absorb the answer.

The recommendations are deliberately **conservative**: when in
doubt, do less in V2.0 and add more in V2.1+ once the cost is
better understood. V2 ships a *bridge*, not a final-shape API.

---

## Q1. Idiom exposure — what to wrap, what to expose

### Question

> Which TLA+ idioms do we want to expose, and which do we want to
> hide behind macros / conveniences? (e.g. the `Init`/`Next`/`Spec`
> boilerplate is repetitive; a Resilient-side template could generate
> it.)

### Recommendation: **expose raw TLA+; auto-generate only for the simplest cases**

V2.0 ships with TLA+ specs as plain `.tla` files referenced from
`@refines` annotations. Authors write `Init`, `Next`, and the
`Spec ≜ Init ∧ ☐[Next]_vars ∧ ⟦fairness⟧` formula by hand. The
Resilient compiler is a *bridge* (per the companion spec's Path
B), not an embedding.

The single auto-generation V2.0 *does* provide is a
`resilient pkg new --tla` scaffolder that emits a minimal `.tla`
template alongside the new module — `Init` and `Next` skeletons
with the right `EXTENDS Naturals, Sequences` boilerplate but no
opinionated wrapping of state transitions. Users edit the
template; we don't try to keep the spec and the impl
auto-synced beyond what `@refines` already enforces.

### Tradeoffs

| Option | Pro | Con |
|---|---|---|
| Expose raw (recommended) | Predictable behaviour, no Resilient-flavored TLA+ dialect, full TLA+ ecosystem (TLA Toolbox, Apalache, learning materials) just works | Boilerplate is repetitive — a 50-line spec has ~15 lines of `EXTENDS` / `vars` / `Spec ≜ …` ceremony |
| Hide behind macros | Less typing per spec, smaller files | We invent a non-standard surface; users debugging counterexamples have to translate Resilient's wrapping back into vanilla TLA+; locks us into supporting the macro language forever |
| Hybrid (some macros, some raw) | Theoretically best of both | In practice, the boundary becomes a learning cliff: "wait, why does *this* invariant need the raw form?" Better to pick one mental model and stick with it |

### Why raw wins

V2 is a tool, not a language extension. The companion spec is
explicit: "We're shipping a *tool*, not a certification claim."
A macro layer would convert "TLA+ specs as artifacts" into
"a Resilient-flavored DSL that compiles to TLA+", which is
a different product with different maintenance cost. The
template scaffolder gives us the ergonomic win (no blank-page
problem) without the maintenance debt of a macro language.

### V2.0 acceptance criteria absorbed

- New: `resilient pkg new --tla` scaffolds a `.tla` template
  alongside the module, with `EXTENDS Naturals, Sequences`,
  empty `Init` / `Next` / `Spec`, and a comment block pointing
  at the `@refines` annotation it should match.
- Removed (was a candidate): no compiler-emitted TLA+ macros
  for `Init`/`Next` boilerplate; no Resilient-side
  `spec { ... }` block wrapping. (Path B already excluded `spec
  { … }`; this just confirms we don't add it under another name.)

---

## Q2. Modeling the runtime — system-under-verification or axioms

### Question

> Do we model the runtime crate (`resilient-runtime`) as part of
> the system being verified, or treat it as an axiomatic library?
> The former is more honest but expensive; the latter requires
> writing TLA+ axioms for every runtime primitive.

### Recommendation: **treat the runtime axiomatically; ship a small `runtime.tla` library of axioms**

V2.0 declares `resilient-runtime` to be the trusted base. The
TLA+ side gets a `runtime.tla` module with axioms covering each
primitive the user can call from a `@refines`'d Resilient
function (initially: actor mailbox semantics, `live { }` block
recovery, scheduler ordering — see Q4 for FFI). User specs
`EXTENDS runtime` and uses the axioms as if they were TLA+
operators.

Auditing the *axioms themselves* is a separate concern, tracked
out of band as part of the runtime crate's review (it's <2k
lines of `#![no_std]` Rust, already unsafe-free, already covered
by RES-220's per-target cross-compile gate). We do **not** model
the runtime's internals in TLA+ — that's where the "expensive"
part of "honest but expensive" lives, and the cost vs. value
ratio doesn't justify it for V2.0.

### Tradeoffs

| Option | Pro | Con |
|---|---|---|
| Model runtime in TLA+ (full system) | Most honest verification — counterexamples expose runtime-side bugs | Costs ≈3–4 contributor-weeks just for the actor scheduler. Every runtime change has a TLA+-mirror update. State space inflates — the scheduler alone adds O(n!) interleavings. Probably blocks V2.0 ship date by months. |
| Axiomatic (recommended) | Fast to ship, axioms are testable in isolation, scope of "what's verified" is unambiguous to users | Trust shifts to the axioms — a wrong axiom proves a wrong invariant. Mitigated by keeping the axiom file small and reviewable. |
| Hybrid (model the actor scheduler, axiomatize the rest) | More honest where it matters most | The scheduler is precisely where modeling cost is highest; this option gets the worst tradeoff |

### Why axioms win

Three reasons:

1. **The runtime is small and stable.** ~2k lines of `#![no_std]`
   Rust with zero new commits in the last 6 months touching the
   actor / live-block / scheduler primitives. Modeling something
   that doesn't change is wasted effort.
2. **The verification cost is bounded by the axioms, not the
   implementation.** A TLA+ check of a user spec proves "given
   these axioms, this invariant holds". That's a meaningful
   guarantee even if the axioms themselves are eyeballed; if
   future work proves the runtime against the axioms, every
   existing spec gets the upgrade for free.
3. **The state-space cost is huge.** The scheduler interleavings
   alone push TLC checking past Q5's "small spec" budget. Users
   hit Apalache-only territory before they finish writing their
   first invariant.

### Open: what's in `runtime.tla`?

Initial axiom set for V2.0:

- `Mailbox`: bounded FIFO with deterministic delivery within a
  single actor, nondeterministic across actors (matches the
  V1 `actor` runtime — see #361 for the formal write-up).
- `LiveRecovery`: at most one user-supplied recovery effect per
  `live { }` block, idempotent under re-entry. The companion
  spec's V1 invariant #2 ("closed-form invariant") makes this
  axiomatically expressible without modeling user code.
- `Scheduler`: weak fairness on every actor (per #361's pending
  decision; if #361 lands strong fairness, the axiom flips).
- Notably **excluded**: heap allocator semantics, MMIO ordering,
  `unsafe` block contents. These are out-of-scope for V2.0
  TLA+ verification; the `@refines` machinery rejects functions
  that touch them with a clear "not modelable" diagnostic.

### V2.0 acceptance criteria absorbed

- New crate: `resilient-tla-stdlib` (or just a `tla/` directory
  in `resilient-runtime/`) shipping `runtime.tla` with the
  axioms above.
- The `@refines` annotation rejects functions whose effect set
  includes anything outside `runtime.tla`'s axiomatic surface,
  pointing the user at the missing axiom.
- Adding a new axiom is a separate ticket workflow with the
  maintainer in the review loop; V2.0 ships with the four
  axioms above and no extension hook for users.

---

## Q3. Counterexample fidelity — what to show

### Question

> Counterexample fidelity — when TLC reports a 12-step trace, does
> the Resilient-side replay show 12 user-meaningful steps, or do
> internal steps of the actor scheduler clutter the output?

### Recommendation: **two-mode replay — `condensed` (default) and `--full-trace`**

V2.2's counterexample replay defaults to a `condensed` view that
shows only state transitions visible at the Resilient surface:
function entries / exits, message sends / receives, user-defined
actions. Internal scheduler steps (preemption decisions, mailbox
internals, weak-fairness step counters) are collapsed into a
single "scheduler ran" placeholder.

`--full-trace` opt-in exposes the raw TLC trace verbatim with
no filtering — for users debugging the spec itself or chasing
a "this can't happen" bug that the condensed view hid.

### Tradeoffs

| Option | Pro | Con |
|---|---|---|
| Always full | Maximum information, no filter logic to maintain | First-time users drown in 200-line traces of `pc[a1] = "..."` updates that don't map to anything in their Resilient code |
| Always condensed | Approachable for the "I just want to know why my invariant failed" case | Power users debugging *the spec* hit a wall — the condensed view filters away the very steps they need |
| Two-mode (recommended) | Default is approachable; opt-in is powerful | Slightly more code (a filter pass) and a UX surface (two flags) to maintain |

### Why two modes wins

Counterexample replay is the moment a user *first* engages with
the TLA+ output. If the default is hostile to first-time users,
they bounce. But power users who write specs for a living also
need raw access. The cost of a filter pass is bounded — a few
hundred lines of code that translate `pc` / `mailbox_internal`
/ `scheduler_step` into either "user-step" or "scheduler-noise"
buckets, then squash adjacent noise.

The flag name `--full-trace` (vs. `--verbose`) is intentional:
"verbose" suggests "more of the same"; "full-trace" signals
"this is a different, lower-level view".

### V2.0 acceptance criteria absorbed

- V2.0 ships only the condensed view (V2.2 is when replay lands;
  V2.0 has no replay yet — but V2.0 does need to know which
  internal vs user-visible step labels to emit *into* the spec
  so V2.2's filter has something to key off).
- V2.0 spec generator labels every TLA+ action with a
  `\* @kind: user | scheduler | mailbox | live | runtime`
  comment. The label is mechanical to add but cheap to forget;
  CI lint enforces it on every action defined inside a `Next`
  formula.
- V2.2's replay reads those labels to bucket steps. `--full-trace`
  ignores them.

---

## Q4. FFI side-effects — `CHOOSE` or contract

### Question

> How does this interact with the FFI? Foreign function calls are
> observably nondeterministic; do we treat them as
> `CHOOSE x \in T : TRUE` or require a contract?

### Recommendation: **contract-required — `extern fn` that lacks `requires`/`ensures` is unmodelable**

V2.0 rejects `@refines` on any function that calls an `extern
fn` lacking both a `requires` and an `ensures` clause. The
diagnostic points at the offending call site and says "this
extern fn has no contract; the TLA+ refinement cannot model
its behaviour". Adding the contract — even one as weak as
`ensures result == result` (i.e. "deterministic for fixed
inputs") — unblocks the refinement.

When a contract *is* present, V2.0 lowers the extern call to
`CHOOSE x \in T : <ensures predicate translated to TLA+>`. So
contracts double as the mechanism that bounds nondeterminism:
a strict contract narrows the `CHOOSE` set; a permissive one
falls back to `CHOOSE x \in T : TRUE`.

### Tradeoffs

| Option | Pro | Con |
|---|---|---|
| Always `CHOOSE x \in T : TRUE` | No new user-facing requirement | Every FFI call inflates the state space combinatorially. Most real specs become uncheckable. Trivial bugs hide because every postcondition is satisfiable by *some* return value. |
| Contract-required (recommended) | State space is bounded by the user's contracts; the cost of imprecision is paid by the author who wants the precision, not by every checker run; aligns with V1's contract culture | Users who paste in a third-party C library now must annotate it before `@refines` works. We get bug reports of the form "I'm not the author of this library, why do I have to write a contract?" |
| Best-effort inference (parse the C header) | Magic when it works | Brittle, header-dependent, can't see across the FFI boundary; this is the "let's add ML to fix it" path. Out of scope. |

### Why contract-required wins

Resilient already requires contracts for verifier-touched code
on the V1 side. Extending that requirement to the V2 surface
is consistent — you opt out of contracts by opting out of
`@refines`, just like you opt out of Z3 by not writing
`requires` / `ensures` on a function today.

The user-facing pain (annotating third-party FFI) is real but
mitigated:

- The compiler can suggest `requires true; ensures true;` as a
  one-line fix when the user just wants the function modelable
  at all (the most permissive contract). The refinement still
  won't *prove* much, but the program builds.
- Common third-party libraries (libc, sqlite, …) get a curated
  contract pack shipped with the runtime. Out of scope for
  V2.0 — tracked as a follow-up, but the architecture doesn't
  block adding it later.

### V2.0 acceptance criteria absorbed

- The `@refines` annotation walker rejects any function reachable
  from an annotated entry that calls an `extern fn` without both
  `requires` and `ensures`. The diagnostic is structured and
  points at the call site, the missing clauses, and the suggested
  one-line fix (`requires true; ensures true;`).
- The TLA+ emitter lowers `extern fn` calls with full contracts
  to `CHOOSE x \in T : <ensures>`; the `T` is the function's
  declared return type, the `<ensures>` is the existing
  ensures-clause translator's output (already shared with the
  Z3 side).
- This is the answer #361 RES-ACTOR-SEMANTICS was waiting for —
  actor handlers calling FFI inherit the same rule.

---

## Q5. Performance — when is TLC enough, when do users need Apalache

### Question

> Performance — TLC is enumerative. What's our expected state-space
> size for a "reasonable" Resilient program, and at what point do we
> tell users to switch to Apalache?

### Recommendation: **TLC default with a 5-min wall-clock budget; auto-suggest Apalache on timeout; ship `benchmarks/tla/` early-V2 to characterize the curve**

V2.0 runs TLC with a 5-minute wall-clock limit by default.
Below the limit: the spec was checkable in TLC, ship.
At the limit: emit a structured diagnostic that says "TLC
explored N states in 5 min and timed out; consider
`--mc-backend apalache` (V2.4) or reducing model parameters
(actor count, mailbox depth, scheduler horizon)".

The 5-min default is a guess. To replace the guess with a
number, V2.0 ships a `benchmarks/tla/` directory with three
representative specs: small (a single actor), medium (3-actor
ring), large (5-actor mesh with `live { }` blocks). The bench
runs on every PR that touches TLA+ infrastructure, the medians
are tracked over time, and the 5-min budget moves up or down
based on what real Resilient programs look like in those
buckets.

### Tradeoffs

| Option | Pro | Con |
|---|---|---|
| No timeout (always run to completion) | Definitive answer | A pathological spec hangs CI forever |
| Hard timeout, fail | Predictable | Frustrating UX when the user has a fixable spec |
| Soft timeout with suggestion (recommended) | Predictable budget, escape hatch documented | A bit more code in the diagnostic emitter |
| Auto-switch to Apalache on timeout | Fully automatic | Apalache is opt-in for V2.4 — auto-switching to a backend that may not be installed locally is bad UX |

### Why soft-timeout-with-suggestion wins

The user shouldn't have to know "TLC vs. Apalache" tradeoffs
upfront. The diagnostic does the teaching: "TLC explored 230k
states in 5 min, didn't terminate; spec_size=large; suggest:
(a) reduce model cardinality, (b) `--mc-backend apalache`
once V2.4 lands". Users learn the boundary by hitting it,
which is when the lesson sticks.

The 5-min figure is from analogy to existing CI gates (the
perf gate runs in <30s; integration tests run in <2min;
`cargo test --features z3` runs in <90s on CI). Doubling the
slowest existing gate gives a budget that's "long enough to
be useful but short enough that a hung run is obviously bad".

### V2.0 acceptance criteria absorbed

- New: `benchmarks/tla/` directory with three benchmark
  specs (small / medium / large) and a `run.sh` that reports
  TLC wall-clock + state-count for each. Wired into a new
  CI gate `tla-perf-gate` that's *informational* in V2.0 (not
  required) — same posture as `perf-gate` is for fib(25).
- New: `TLA_TIMEOUT_SECS` env var (default 300) controls the
  per-spec wall-clock limit. CI uses 600 to absorb runner
  variance; local default is 300.
- New: structured diagnostic on timeout. The diagnostic is a
  single sentence with the state count, the timeout, the
  spec size bucket (small/medium/large from a heuristic
  `wc -l <.tla>`), and a one-line suggestion.
- V2.4 gets the auto-switch hook: `--mc-backend apalache` does
  not yet exist in V2.0; the diagnostic mentions it as a
  forward-looking option.

---

## Sign-off summary

| # | Question | Recommendation | Risk if wrong |
|---|---|---|---|
| Q1 | Idiom exposure | Expose raw TLA+; scaffolder for blank-page problem | Low — easy to add macros later |
| Q2 | Runtime modeling | Axiomatic, ship `runtime.tla` | Medium — switching to full modeling later means re-doing every existing spec |
| Q3 | Counterexample fidelity | Two-mode (condensed default, `--full-trace` opt-in) | Low — filter pass is small, removable |
| Q4 | FFI semantics | Contract-required; lower to `CHOOSE x \in T : <ensures>` | High — flips would break every existing spec; this needs to be locked before V2.0 |
| Q5 | Performance budget | TLC + 5-min soft timeout + benchmarks/tla/ | Low — the budget is data-driven, easy to retune |

**Recommended action**: maintainer accepts this sub-spec
unchanged or with adjustments; on accept, the V2.0 / V2.1 /
V2.2 implementation tickets fold each Q's "V2.0 acceptance
criteria absorbed" section back into their own acceptance
criteria. With sign-off complete, #373 closes and the V2.0
implementation work is unblocked. #361 RES-ACTOR-SEMANTICS
specifically consumes Q4's recommendation as its own
acceptance criterion — it can move out of `blocked` once
this lands.

---

## What this spec does NOT decide

- The exact TLA+ vocabulary in `runtime.tla` (Q2 names four
  axioms; their precise temporal-logic encoding is a V2.0
  implementation detail that lands with the file).
- Whether `--mc-backend apalache` (V2.4) requires Apalache
  installed locally vs. a cloud-side model checker. That's
  a V2.4 decision; V2.0 / V2.2 don't need it.
- The shape of the V2.1 `@refines` annotation grammar. The
  companion spec's V2.1 phasing covers it; this sub-spec
  doesn't extend or contradict that.
- Any decision about V3+ (full-system modeling, embedded
  `spec { … }`, dependent types). V2 is a bridge; V3 is its
  own conversation.
