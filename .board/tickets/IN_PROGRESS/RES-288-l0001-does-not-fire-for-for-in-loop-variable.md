---
id: RES-288
title: "L0001: lint does not fire for unused `for-in` loop variable"
state: IN_PROGRESS
priority: P3
goalpost: G14
created: 2026-04-20
owner: executor
claimed_by: Claude
---

## Summary

The L0001 ("unused local binding") lint tracks `let` and `static let`
bindings via `collect_lets_in` in `lint.rs`. However,
`Node::ForInStatement` introduces a new local binding via its `name`
field (the loop iterator variable), and `collect_lets_in` ignores it:

```rust
Node::ForInStatement { iterable, body, .. } => {
    collect_lets_in(iterable, out);
    collect_lets_in(body, out);
    // `name` (the loop variable) is swallowed by `..` — never registered
}
```

As a result, unused loop variables produce no L0001 warning:

```resilient
fn f() {
    let arr = [1, 2, 3];
    for item in arr {   // `item` is bound but never used
        return 1;
    }
}
```

The `item` binding should trigger L0001 but does not.

## Affected code

- `resilient/src/lint.rs` — `collect_lets_in`, the `Node::ForInStatement`
  arm (around line 200): uses `..` to ignore the `name` field.

## Acceptance criteria

- `collect_lets_in` for `Node::ForInStatement` registers the `name`
  field as a binding: `out.push((name.clone(), *span))`.
- Underscore-prefixed loop variables (`_item`) are suppressed per the
  existing rule.
- New unit tests in `lint.rs`:
  - `l0001_fires_on_unused_for_in_loop_variable` — loop variable never
    used in body.
  - `l0001_silent_when_for_in_loop_variable_is_used` — loop variable
    used in body.
  - `l0001_silent_for_underscore_prefixed_for_in_variable` — `_item`
    is exempt.
- Existing L0001 tests continue to pass.
- `cargo test` passes with 0 failures.
- `cargo clippy --all-targets -- -D warnings` clean.
- Commit: `RES-288: L0001 fires on unused for-in loop variable`.

## Notes

- The span to attach to the binding should be the span of the
  `ForInStatement` node (the `span` field). A more precise
  "variable-name span" is a follow-up if the AST ever gains one.
- The `collect_identifier_reads_in` function already correctly
  descends into `body` for `ForInStatement`; only the declaration
  side (`collect_lets_in`) needs updating.
- This is a companion to RES-259 (match-arm bindings); both add
  let-equivalent bindings that L0001 currently misses.
- This is an additive lint improvement — no existing test should
  fire where it did not before.

## Log

- 2026-04-20 created by analyzer (`collect_lets_in` ignores
  `ForInStatement.name`; L0001 has false negatives for loop variables)
