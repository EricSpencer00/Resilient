---
id: RES-232
title: "L0002 lint: `x @ _` bind-pattern not treated as catch-all for unreachable-arm detection"
state: IN_PROGRESS
priority: P3
goalpost: tooling
created: 2026-04-20
owner: executor
claimed-by: Claude
---

## Summary

The L0002 ("unreachable arm after `_`") lint in `resilient/src/lint.rs` uses a
bare `matches!(pat, Pattern::Wildcard)` check to determine whether an arm is a
catch-all. Since RES-161a landed `Pattern::Bind`, a bind pattern whose inner
pattern is a default (e.g. `x @ _`, `x @ name`) also matches every value and
makes subsequent arms unreachable — but the lint does not flag them.

The typechecker already has a correct `pattern_is_default` helper (in
`resilient/src/typechecker.rs`, line 127) that recurses through `Pattern::Bind`
and `Pattern::Or` to determine defaultness. The lint needs to use the same
logic.

## Affected location

`resilient/src/lint.rs`, function `walk_matches`, inside the `Node::Match` arm:

```rust
// Current (broken):
if matches!(pat, Pattern::Wildcard) {
    saw_wild = true;
}

// Should be (using the typechecker helper or an equivalent inline check):
if pattern_is_default_for_lint(pat) {
    saw_wild = true;
}
```

where `pattern_is_default_for_lint` mirrors `typechecker::pattern_is_default`:

```rust
fn pattern_is_default_for_lint(p: &Pattern) -> bool {
    match p {
        Pattern::Wildcard | Pattern::Identifier(_) => true,
        Pattern::Literal(_) => false,
        Pattern::Or(branches) => branches.iter().any(pattern_is_default_for_lint),
        Pattern::Bind(_, inner) => pattern_is_default_for_lint(inner),
    }
}
```

## Reproducer

```
// resilient source — no L0002 warning emitted today, but the second arm is
// unreachable:
match x {
    n @ _ => 1,   // catch-all — matches everything
    0     => 2,   // unreachable, but L0002 is silent
}
```

## Impact

- Silent correctness issue in user code: dead match arms are not flagged.
- Newly introduced by RES-161a; `Pattern::Bind` didn't exist before that PR.

## Acceptance criteria

- `pattern_is_default_for_lint` (or a renamed shared function) is added to
  `lint.rs` and used in `walk_matches` to set `saw_wild`.
- The following inputs each emit an L0002 warning on the unreachable arm:
  - `match x { n @ _ => 1, 0 => 2, }` — bind with wildcard inner
  - `match x { n @ m => 1, 0 => 2, }` — bind with identifier inner
  - `match x { _ => 1, 0 => 2, }` — existing wildcard case still fires
- The following inputs do NOT emit L0002:
  - `match x { n @ 5 => 1, 0 => 2, }` — bind with literal inner (not catch-all)
- Add unit tests in `lint.rs` covering each of the above cases.
- `cargo test` remains fully green.
- `cargo clippy --all-targets -- -D warnings` remains clean.
- Commit message: `RES-232: L0002 lint recognises Pattern::Bind as a catch-all`.

## Notes

- Do NOT modify existing tests — add only new ones.
- The `pattern_is_default` function in `typechecker.rs` is the ground truth.
  Consider extracting it to a shared location (e.g. `resilient/src/patterns.rs`)
  if the duplication grows; for this ticket, a local copy in `lint.rs` is fine.
- `Pattern::Identifier` (bare name binding) was also a pre-existing gap but is
  explicitly out of scope for this ticket to keep the change minimal.

## Log
- 2026-04-20 created by analyzer
