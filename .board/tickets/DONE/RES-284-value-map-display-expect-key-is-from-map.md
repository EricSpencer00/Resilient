---
id: RES-284
title: "Value::Map Display impl uses .expect(\"key is from map\") in production fmt code"
state: DONE
Claimed-by: Claude Sonnet 4.6
priority: P4
goalpost: G3
created: 2026-04-20
owner: executor
---

## Summary

The `Display` implementation for `Value::Map` in `resilient/src/main.rs`
(inside the `impl std::fmt::Display for Value` block) contains a
`.expect()` call in production library code:

```rust
for (i, k) in keys.iter().enumerate() {
    if i > 0 {
        write!(f, ", ")?;
    }
    write!(f, "{} -> {}", k, m.get(k).expect("key is from map"))?;
}
```

The invariant holds: `keys` is collected from `m.keys()`, so every `k`
is guaranteed to be present. The `.expect()` cannot panic in correct
single-threaded code. However, it is an `.expect()` in production library
code (not test code and not `main()` setup logic), which is inconsistent
with the no-panic policy in CLAUDE.md.

## Affected code

`resilient/src/main.rs` — `impl std::fmt::Display for Value`, the
`Value::Map` arm, line ~4814:

```rust
m.get(k).expect("key is from map")
```

## Acceptance criteria

Replace `.expect("key is from map")` with the `entry`-API or, since the
key is known-present, with a direct `m[k]` index (which panics on missing
key with a cleaner message and is idiomatic for confirmed-present keys) or
`m.get(k).unwrap_or_else(|| unreachable!("HashMap key invariant broken"))`.

The preferred fix is using Rust's `Index` trait (`&m[k]`) which is
idiomatic for "I am certain this key exists":

```rust
write!(f, "{} -> {}", k, &m[k])?;
```

- No change in output for any map value.
- `cargo test` passes with 0 failures.
- `cargo clippy --all-targets -- -D warnings` clean.
- Commit: `RES-284: Value::Map Display — replace expect("key is from map") with &m[k]`.

## Notes

- This is a cosmetic/defensive change only. No behaviour change.
- `Value::Map` is the `HashMap<MapKey, Value>` type; `m[k]` calls
  `Index::index` which panics with `"key not found"` if absent — but we
  know it cannot be absent, so the semantics are identical to today's
  `.expect()` but expressed idiomatically.
- Do NOT change any test or golden file.

## Log

- 2026-04-20 created by analyzer (main.rs Value::Map Display arm uses
  .expect("key is from map") in production fmt code; invariant is sound
  but idiom is inconsistent with no-panic policy for library code)
