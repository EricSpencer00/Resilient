---
id: RES-277
title: "ffi.rs: production .expect(\"inserted above\") in load_library — replace with unreachable!"
state: OPEN
priority: P4
goalpost: G3
created: 2026-04-20
owner: executor
---

## Summary

`resilient/src/ffi.rs` (the `ForeignLoader::load_library` method) contains a
production `.expect()` call:

```rust
if !self.libs.contains_key(library) {
    let lib = unsafe { libloading::Library::new(library) }.map_err(|e| { ... })?;
    self.libs.insert(library.to_string(), lib);
}
let lib = self.libs.get(library).expect("inserted above");
```

The invariant is sound: the `if` branch always inserts before the `.get()`,
and the `else` branch (skip insert) only runs when the key already exists.
The `.expect("inserted above")` therefore cannot panic in correct code.

However, it is still a `.expect()` in production library code that would
panic if:
1. The underlying `HashMap::get` ever returns `None` due to a future
   refactor (e.g., moving the insert behind a flag or restructuring the
   branches).
2. A fuzz harness calls `load_library` in a multi-threaded context where
   someone else removes the entry between the insert and the get.

The idiomatic replacement is `unreachable!()` (which documents the invariant
as a compile-verified assertion) or the `entry()` / `or_insert_with` API
which eliminates the double-lookup entirely.

## Affected code

`resilient/src/ffi.rs` — `ForeignLoader::load_library`, line ~245:

```rust
let lib = self.libs.get(library).expect("inserted above");
```

## Acceptance criteria

**Option A** (preferred): Use the `entry` API to collapse the
`contains_key` check, `insert`, and `get` into a single lookup:

```rust
let lib = match self.libs.entry(library.to_string()) {
    std::collections::hash_map::Entry::Occupied(e) => e.into_mut(),
    std::collections::hash_map::Entry::Vacant(e) => {
        let lib = unsafe { libloading::Library::new(library) }.map_err(|e| {
            FfiError::LibNotFound { ... }
        })?;
        e.insert(lib)
    }
};
```

**Option B**: Replace `.expect("inserted above")` with
`unreachable!("ffi: load_library: HashMap::get returned None after insert")`.

Either option is acceptable. Option A is preferred because it eliminates
the double-lookup and makes the invariant structurally impossible to violate.

- All existing FFI tests pass unchanged.
- `cargo test` passes with 0 failures.
- `cargo clippy --all-targets -- -D warnings` clean.
- Commit: `RES-277: ffi load_library — replace expect("inserted above") with entry API`.

## Notes

- This is purely defensive: the existing invariant is correct today.
- The `ffi` feature is gated (`--features ffi`); CI must run with that
  feature enabled to exercise this code path.
- Do NOT change any test or golden file.
- If Option A is chosen, the `if !self.libs.contains_key(library) { ... }`
  block and the subsequent `self.libs.get(library).expect(...)` line are
  both replaced by the `entry` block.

## Log

- 2026-04-20 created by analyzer (ffi.rs load_library uses .expect("inserted above")
  in production code; structurally sound today but violates no-panic policy in
  CLAUDE.md for library code; entry API is the idiomatic fix)
