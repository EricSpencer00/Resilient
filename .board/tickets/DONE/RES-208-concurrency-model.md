---
id: RES-208
title: Concurrency model — actor/supervisor design on top of effects (G18)
state: DONE
priority: P3
Claimed-by: Claude Sonnet 4.6
goalpost: G18
created: 2026-04-18
owner: executor
---

## Summary
Resilient is single-threaded today across all three backends
(interpreter, VM, JIT) and across the `#![no_std]` embedded
runtime. That's been honest-but-incomplete: the philosophy
page gestures at Erlang-style supervisor trees, several open
tickets (RES-191 `@pure`, RES-192 `@io` inference, RES-193
effect polymorphism) collectively build the G18 effect
system, and the safety-standard story benefits from having a
concrete concurrency model to argue against. This ticket is
the design-and-scaffolding work: nail down the actor/message
semantics on paper, identify the prerequisite tickets, and
produce the first landable increment (parser + AST for a
future `actor` / `spawn` / `send` / `recv` surface, behind a
feature flag, with no runtime).

The user-facing concurrency doc now exists at
`docs/concurrency.md` — this ticket is the engineering work
that doc points at.

## Acceptance criteria
- Design note at `.board/designs/concurrency.md` (new
  directory — the `.board/` convention permits it; first
  design note lives here) covering:
  - Actor semantics: state ownership, message typing,
    `spawn` / `send` / `recv` primitive shapes.
  - Supervisor semantics: restart policies
    (`one_for_one`, `one_for_all`, `rest_for_one`
    borrowed from OTP), exit propagation, how a crashed
    actor composes with `live { }` retry budgets.
  - Scheduler model: cooperative, yield points, interaction
    with host RTOS (FreeRTOS / Zephyr task == one Resilient
    program, actors multiplexed inside it).
  - Effect-system prerequisites: which rows in the G18
    lattice (`@pure`, `@io`, `@random`, a future
    `@actor`?) a concurrent program relies on, and the
    soundness rule that forbids shared mutable state.
  - Non-goals: preemption of Resilient code by Resilient
    code, shared-memory concurrency, unbounded work-
    stealing, `unsafe` atomics.
- Parser / AST scaffolding behind a feature flag
  `concurrency-preview` (no runtime, parse-only):
  - `actor <Name> { ... }` parses to a new `Node::ActorDecl`.
  - `spawn <Name>` parses to a new `Node::Spawn` expression.
  - `send <expr>, <expr>` parses to a new `Node::Send`.
  - `recv <expr>` parses to a new `Node::Recv`.
  - Behind the feature flag, the typechecker emits a
    `warning[W0001]: concurrency preview feature used,
    semantics are not stable` diagnostic and the interpreter
    / VM / JIT refuse to execute these nodes with a clear
    error.
- Unit tests: parser accepts all four forms; with the
  feature flag off, they are parse errors with a helpful
  diagnostic pointing at the flag.
- Docs: `docs/concurrency.md` gets a "Preview flag" note
  linking to this ticket and warning that the syntax is
  not stable.
- Commit message: `RES-208: concurrency preview — parser
  + AST scaffolding, design note`.

## Notes
- **Blocked on G18's first concrete landing.** At minimum
  RES-191 (`@pure` annotation) should land before this
  ticket's runtime work starts, since the actor soundness
  rule needs a real purity predicate to check. The
  parse-only scaffolding in this ticket is independent of
  G18 and can land today.
- **Do not wire up any scheduler in this ticket.** The
  scheduler is the next ticket (RES-209 when minted). This
  one is design + parse + reject-at-runtime only. Landing a
  half-scheduler invites shipping bugs downstream of the
  design note being wrong.
- **The actor syntax in `docs/concurrency.md` is explicitly
  labeled speculative.** This ticket's design note either
  ratifies that sketch or revises it — in either case, the
  docs page's speculative section must be updated to match
  what this ticket's design concludes, before the ticket is
  marked DONE.
- **Number-assignment note.** The user-facing concurrency
  doc went in under a request that specified RES-207 for
  this ticket. RES-207 was already DONE (tutorial series);
  this ticket took the next free id (RES-208) instead.
  `docs/concurrency.md` links back to "the roadmap" without
  naming a specific id, so no doc needs to change.

## Log
- 2026-04-18 created by manager (alongside `docs/concurrency.md`)
