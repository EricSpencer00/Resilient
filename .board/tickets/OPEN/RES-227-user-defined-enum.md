---
id: RES-227
title: User-defined `enum` with data variants (tagged unions)
state: OPEN
priority: P2
goalpost: G7
created: 2026-04-20
owner: executor
---

## Summary
Add user-defined `enum` types with data-carrying variants — a full algebraic sum type beyond the built-in `Result<T, E>`. Unlocks idiomatic modelling of state machines, error hierarchies, and optional values.

```
enum Shape {
    Circle { radius: Float },
    Rect { width: Float, height: Float },
    Point,
}
```

## Acceptance criteria
- **Syntax**: `enum Name { Variant, Variant { field: T, ... }, ... }` parsed and stored in the AST.
- **Construction**: `Shape::Circle { radius: 1.5 }` produces an enum value.
- **Pattern matching**: `match` arms destructure enum variants by name and bind fields.
- **Exhaustiveness**: the existing exhaustiveness checker covers enum variants — missing arm is a compile error.
- **Type checker**: match arms must all return the same type; variant field types unify with declared types.
- **Unit variants** (no payload) are valid.
- At least two golden tests: a `Shape` area computation and a state-machine example.
- Commit message: `RES-227: user-defined enum with data variants and exhaustiveness checking`.

## Notes
- The `Result` built-in should not be broken by this change.
- Runtime representation can be a tagged struct `{ tag: usize, payload: HashMap<String, Value> }` initially.
- Large ticket — split off sub-tickets if the PR grows past ~400 lines.

## Log
- 2026-04-20 created by manager
</content>
</invoke>