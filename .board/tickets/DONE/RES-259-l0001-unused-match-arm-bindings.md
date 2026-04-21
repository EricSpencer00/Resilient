---
id: RES-259
title: "L0001: lint does not fire for unused match-arm bindings"
state: DONE
priority: P3
goalpost: G14
created: 2026-04-20
owner: executor
Claimed-by: Claude
---

## Summary

The L0001 ("unused local binding") lint tracks `let` and `static let`
bindings via `collect_lets_in` in `lint.rs`. However, match-arm
`Pattern::Identifier` bindings — which introduce a new local name bound
to the matched scrutinee — are never added to the `lets` list. As a result,
unused match-arm bindings produce no warning.

Example that should trigger L0001 but does not:

```resilient
fn f() {
    let x = 5;
    match x {
        y => { return 1; }   // `y` is bound but never used
    }
}
```

The `y` binding is scoped to the arm body, but since `collect_lets_in`
does not descend into `Pattern::Identifier` arms, L0001 never sees it.
The same gap exists for `Pattern::Bind` (the `name` part of `name @ inner`).

## Affected code

- `resilient/src/lint.rs` — `collect_lets_in` (line ~163): the
  `Node::Match` arm only recurses into the arm body; it does not inspect
  `Pattern::Identifier(name)` or `Pattern::Bind(name, _)` to register
  new "let-equivalent" bindings.

## Acceptance criteria

- `collect_lets_in` for `Node::Match` inspects each arm's pattern:
  - `Pattern::Identifier(name)` → push `(name, arm_span)` as a binding.
  - `Pattern::Bind(name, _)` → push `(name, arm_span)` as a binding.
  - `Pattern::Wildcard` and `Pattern::Literal` → no binding (already
    correct).
  - `Pattern::Or(branches)` → collect bindings from the first branch
    (all branches bind the same names per the parser invariant).
- The scope for each arm binding is restricted to that arm's body: the
  used-identifier collection over the arm body is the correct scope
  (the existing `collect_identifier_reads_in` over `arm_body` already
  handles this — only the "declaration" side needs updating).
- Underscore-prefixed names (`_x`) are suppressed per the existing rule.
- New unit tests in `lint.rs`:
  - `l0001_fires_on_unused_match_arm_binding` — pattern identifier never
    used in arm body.
  - `l0001_silent_when_match_arm_binding_is_used` — binding used in body.
  - `l0001_silent_for_underscore_prefixed_match_arm_binding`.
  - `l0001_fires_on_unused_bind_pattern_name` — the `name` in `name @ _`.
- Existing L0001 tests continue to pass.
- `cargo test` passes with 0 failures.
- `cargo clippy --all-targets -- -D warnings` clean.
- Commit: `RES-259: L0001 fires on unused match-arm bindings`.

## Notes

- Match-arm span: use the span of the enclosing `Node::Match` or the
  arm body for the lint position (a more precise per-arm span is a
  follow-up if spans ever become available on individual arms).
- Do NOT count `Pattern::Wildcard` as a binding — `_` is the explicit
  discard and should remain silent.
- `Pattern::Or`: all branches bind the same names; reading the first
  branch's bindings is sufficient.
- This is an additive lint improvement — no existing test should fire
  where it did not before. The only change is new warnings on previously-
  unchecked patterns.

## Log

- 2026-04-20 created by analyzer (`collect_lets_in` does not inspect
  Pattern::Identifier/Bind in match arms; L0001 has false negatives)
- 2026-04-20 closed by Claude — commit aaf6c94; all 4 acceptance-criteria
  tests pass; 723 total tests green; clippy and fmt clean.
