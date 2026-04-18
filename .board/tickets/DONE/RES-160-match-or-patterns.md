---
id: RES-160
title: Or-patterns in match arms `case 0 | 1 | 2 => ...`
state: DONE
priority: P3
goalpost: G13
created: 2026-04-17
owner: executor
---

## Summary
Or-patterns collapse clusters of equivalent arms into one line.
Exhaustiveness treats them as the union of the covered spaces,
which is natural in the coalescing algorithm from RES-129.

## Acceptance criteria
- Parser: `<pattern> | <pattern> | ...` at the top of a match arm.
  Lower precedence than struct/tuple destructuring — parens required
  to combine.
- Bindings: if any branch binds a name, ALL branches must bind the
  same set of names to the same types. Otherwise a typecheck
  error: `or-pattern branches bind different names`.
- Exhaustiveness: union the covered space of each branch.
- Unit tests: numeric or-pattern, string or-pattern, mismatched
  bindings error.
- Commit message: `RES-160: or-patterns in match arms`.

## Notes
- Same-binding constraint matches Rust's semantics and avoids
  user confusion about "which branch was taken".
- Don't support or-patterns in `let` bindings yet — match-only
  surface for this ticket.

## Log
- 2026-04-17 created by manager
- 2026-04-17 claimed by executor
- 2026-04-17 done by executor

## Resolution
- `resilient/src/main.rs`:
  - New `Pattern::Or(Vec<Pattern>)` variant.
  - Parser: `parse_pattern` wraps a new `parse_pattern_atom` and
    collects `| <pattern>` tails (via `Token::BitOr`) into an
    `Or`. `|` is unambiguous in pattern position since no atomic
    pattern starts with `|`. Single-atom patterns are returned
    un-wrapped, so existing arms keep their exact AST shape.
  - Interpreter `match_pattern` gains an `Or` arm — first branch
    that matches wins, returning its binding (always consistent
    thanks to the typechecker check). Falls through cleanly when
    no branch matches.
- `resilient/src/typechecker.rs`:
  - Four new helpers next to `compatible`:
    - `pattern_bindings(p) -> Vec<String>`
    - `pattern_single_binding(p) -> Option<String>` — surfaces
      the shared name for Or-patterns so match-arm scoping
      (RES-159) keeps working transparently
    - `pattern_is_default(p)` — recurses through Or for the
      exhaustiveness check; `0 | _` still counts as a default
    - `pattern_covers_bool(p, want)` — recurses so
      `true | false` covers both branches
  - Match arm handling rejects or-patterns whose branches bind
    different names with the ticket's required diagnostic shape
    (`"or-pattern branches bind different names: [...] vs [...]"`).
  - Exhaustiveness switched from the ad-hoc `matches!` check to
    the helpers, so Or-patterns behave correctly across both
    the default-arm and bool-coverage paths.
- `SYNTAX.md`: new "Or-patterns (RES-160)" subsection with
  weekday/weekend, bool-coverage, and the same-bindings rule.
- Deviations: none.
- Unit tests (7 new):
  - `or_pattern_int_any_branch_matches` — weekday classifier
  - `or_pattern_string_any_branch_matches`
  - `or_pattern_mismatched_bindings_error` — ticket AC for the
    diagnostic
  - `or_pattern_bool_both_branches_is_exhaustive` —
    `true | false` no `_` needed
  - `or_pattern_wildcard_branch_counts_as_default` — `0 | _`
    covers even int scrutinees
  - `or_pattern_all_branches_bind_same_name_is_valid` — `x | x`
    is permitted and binding is visible in the body
  - `or_pattern_no_match_falls_through` — misses drop to the
    next arm
- Verification:
  - `cargo test --locked` — 435 passed (was 428 before RES-160)
  - `cargo test --locked --features logos-lexer` — 436 passed
  - `cargo clippy --locked --features logos-lexer,z3 --tests
    -- -D warnings` — clean
