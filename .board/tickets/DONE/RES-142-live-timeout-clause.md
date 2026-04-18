---
id: RES-142
title: `live ... within 10ms` wall-clock timeout clause
state: DONE
priority: P3
goalpost: G10
created: 2026-04-17
owner: executor
---

## Summary
Retry forever is sometimes the wrong semantics — a control loop
would rather fail safely than re-spin forever. Add an optional
`within <duration>` clause that caps total time (retries
included) inside a live block. On expiry, the block escalates
exactly like exhaustion (RES-140).

## Acceptance criteria
- Syntax: `live within 10ms { ... }`, `live within 100us { ... }`.
  Duration literal is `<integer><unit>` where unit ∈ {`ns`, `us`,
  `ms`, `s`}. New `DurationLiteral` AST node.
- Runtime: take `Instant::now()` on block entry; before each retry,
  check elapsed vs budget; if over, treat as exhaustion.
- Interacts cleanly with `backoff(...)` (RES-139) — backoff sleeps
  count against the budget.
- no_std build uses the same clock abstraction placeholder as
  RES-139.
- Unit tests (std): tight inner body + 10ms budget exhausts; slack
  budget succeeds.
- Commit message: `RES-142: live within <duration> timeout clause`.

## Notes
- Duration literals are not a full time library — they only exist
  inside live clauses for now. Don't generalize.
- Combined syntax: `live backoff(...) within 50ms { ... }` — both
  clauses present in either order. Pin the order the parser
  expects and document.

## Log
- 2026-04-17 created by manager
- 2026-04-17 claimed by executor
- 2026-04-17 done by executor

## Resolution
- `resilient/src/main.rs`:
  - New `Node::DurationLiteral { nanos: u64, span: Span }` AST
    variant (per ticket bullet "New DurationLiteral AST node").
    Parser-internal; rejected with a dedicated diagnostic if it
    reaches `eval` outside a `live ... within ...` clause.
  - `Node::LiveBlock` gains a `timeout: Option<Box<Node>>` field
    that carries the parsed duration literal.
  - `parse_live_block` now loops over `backoff(...)` and `within
    <duration>` prefix clauses in either order, rejecting
    duplicates. New helper `parse_within_clause` consumes
    `within <integer><unit>` where `unit ∈ {ns, us, ms, s}` and
    packs it as a `Node::DurationLiteral`, with saturating
    unit-to-ns multiplication to guard against absurd inputs.
  - `eval_live_block` gains a `timeout_ns: Option<u64>`
    parameter and samples `Instant::now()` on entry when a
    budget is present. Before every retry (and thus before the
    backoff sleep) it compares elapsed nanoseconds against the
    budget; on overflow it escalates identically to
    exhaustion — `LIVE_TOTAL_EXHAUSTIONS` bumps (RES-141),
    `(retry depth: N)` footer fires (RES-140) — and returns
    the error with a distinct `"Live block timed out"` prefix
    so users can tell retry-cap from wall-clock failure.
  - Backoff and timeout coexist: sleeps count against the
    budget (tested by the tight-budget exhaustion case).
- `resilient/src/typechecker.rs`: added a `Node::DurationLiteral`
  arm returning `Type::Int` — defensive; the typechecker's
  outer match is exhaustive, so the new variant needs
  coverage.
- `resilient/src/compiler.rs`: added a `Node::DurationLiteral`
  span accessor arm in `node_line` (the VM dispatcher's
  exhaustive span match).
- `SYNTAX.md` extended with a "Wall-clock timeout (RES-142)"
  subsection under Live Blocks. Documents both-order
  acceptance, how backoff interacts with the budget, the
  `"timed out"` diagnostic prefix, and the no_std clock-
  placeholder deviation noted below.
- Deviations:
  - The no_std / embedded runtime clock hook is left for a
    follow-up, matching RES-139's precedent — the ticket's
    acceptance explicitly says "same clock abstraction
    placeholder as RES-139". The std path (used by all tests)
    is fully wired; embedded targets ignore the clause.
- Unit tests (in `main.rs` test module):
  - `parse_live_within_ms_populates_timeout`
  - `parse_live_within_each_unit` — asserts the ns/us/ms/s
    conversion table
  - `parse_live_unknown_duration_unit_errors`
  - `parse_live_duration_requires_nonneg_int`
  - `parse_live_within_both_orders_accepted` — both clause
    orderings in ticket Notes
  - `parse_live_duplicate_within_errors`
  - `parse_live_without_within_keeps_none`
  - `live_within_tight_budget_exhausts_with_timeout_prefix`
    — forces a timeout with 2ms backoff and a 1ms cap,
    asserts the `"Live block timed out"` prefix
  - `live_within_slack_budget_succeeds` — 1s budget on a
    2-fail-then-succeed body; clean success
  - `duration_literal_in_expression_position_is_rejected` —
    defensive eval-path guard
- Verification:
  - `cargo test --locked` — 327 passed (was 317 before RES-142)
  - `cargo test --locked --features logos-lexer` — 328 passed
  - `cargo clippy --locked --features logos-lexer,z3 --tests
    -- -D warnings` — clean
