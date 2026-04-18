---
id: RES-159
title: Match arm guards `case X(n) if n > 0 => ...`
state: DONE
priority: P3
goalpost: G13
created: 2026-04-17
owner: executor
---

## Summary
Today match arms match on shape only. Adding a guard — a boolean
expr gated by `if` — lets users express "first arm whose pattern
matches AND whose guard is true". The exhaustiveness checker has
to back off politely: guarded arms don't count as covering their
pattern.

## Acceptance criteria
- Parser: `case <pattern> if <expr> => <body>`.
- Semantics: guard evaluated in the pattern's scope (so pattern
  bindings are visible). False guard → fall through to next arm.
- Exhaustiveness (RES-054): a guarded arm is treated as
  non-covering; its pattern is still considered "partially
  covered" (so a following `case _ => ...` is not flagged as
  unreachable).
- Unit tests: guard binding access, false guard falls through,
  exhaustiveness behavior with and without unguarded catch-all.
- Commit message: `RES-159: match arm guards`.

## Notes
- Guard expressions that call impure functions or mutate state:
  allowed, but strongly cautioned against in SYNTAX.md. The
  verifier (G9) will refuse to reason about them.
- No `@` bindings yet — that's RES-161.

## Log
- 2026-04-17 created by manager
- 2026-04-17 claimed by executor
- 2026-04-17 done by executor

## Resolution
- `resilient/src/main.rs`:
  - `Node::Match::arms` field reshape:
    `Vec<(Pattern, Node)>` → `Vec<(Pattern, Option<Node>, Node)>`.
    The middle element is the optional guard expression.
  - `parse_match_expression`: between the pattern and `=>`, if
    `current_token == Token::If`, consume it and parse an
    expression as the guard. Unguarded arms carry `None`.
  - Interpreter eval arm: opens the pattern's binding scope once,
    evaluates the guard (if any) inside that scope, and only
    runs the body when the guard is truthy or absent. A false
    guard falls through to the next arm via `continue`; the
    scoped env is rolled back on every exit path so the binding
    never leaks regardless of whether the guard fired.
- `resilient/src/typechecker.rs`:
  - `TypeEnvironment::remove(name)` — new helper to roll back
    transient bindings in the current scope (leaves outer-scope
    bindings intact).
  - Match arm handling now registers the pattern's
    `Pattern::Identifier(n)` binding into the env with the
    scrutinee's type while the guard + body are checked, then
    rolls back. This matches the interpreter's scoping and fixes
    a latent gap where bodies couldn't reference identifier
    bindings either.
  - Guard expressions must typecheck to `Bool` (or `Any`); other
    types error with "Match arm guard must be a boolean".
  - Exhaustiveness check tightened: `has_default` ignores
    guarded wildcard / identifier arms (a guarded catch-all
    might not fire), and the bool-coverage check now requires
    the `true` / `false` arms to be **unguarded**. This is the
    ticket's "a guarded arm is treated as non-covering; its
    pattern is still considered partially covered (so a
    following `case _ => ...` is not flagged as unreachable)".
- `SYNTAX.md`: new "Match expressions" + "Arm guards" sections
  with the classify-by-sign canonical example, the ticket's
  rule about guarded arms not counting toward exhaustiveness,
  and the warning that the verifier won't reason about impure
  guards (ticket Notes).
- Deviations: none. The typechecker's scoping fix for
  identifier-pattern bindings is an incidental improvement
  that falls out of the guard-scoping work — guards demanded
  it, and bodies benefit for free.
- Unit tests (8 new):
  - `match_guard_true_body_fires`
  - `match_guard_false_falls_through`
  - `match_guard_has_access_to_pattern_binding` — classify by
    sign across negative / zero / 42 / 999
  - `match_guard_binding_does_not_leak_outside_arm`
  - `match_guarded_catchall_does_not_count_as_exhaustive` —
    ticket AC for non-exhaustiveness on guard-only match
  - `match_guarded_bool_arms_still_require_both_sides` —
    bool coverage rule
  - `match_guarded_then_unguarded_wildcard_is_exhaustive` —
    the canonical "guard then fallback" shape passes
  - `match_non_boolean_guard_is_typecheck_error`
- Verification:
  - `cargo test --locked` — 428 passed (was 420 before RES-159)
  - `cargo test --locked --features logos-lexer` — 429 passed
  - `cargo clippy --locked --features logos-lexer,z3 --tests
    -- -D warnings` — clean
