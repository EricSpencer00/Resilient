---
id: RES-270
title: "peephole.rs: expect() panic in jump-relinking loop — replace with Result"
state: OPEN
priority: P3
goalpost: G3
created: 2026-04-20
owner: executor
---

## Summary

`resilient/src/peephole.rs` line 130 contains a production `.expect()` call
inside the jump-offset re-linking loop of the peephole optimizer:

```rust
let old_pc = (0..chunk.code.len())
    .find(|&p| old_to_new[p] == new_pc)
    .expect("every new_pc has an originating old_pc");
```

This is production code (called from the compiler pipeline via
`bytecode::optimize`), not test code or `main()` setup logic. CLAUDE.md
states: "A panic in the compiler is a bug."

The invariant holds today because the rewriting loop emits exactly one new op
per old op (never synthesizes extra ops). However:

1. A future peephole rule that *inserts* a new jump op (e.g., for loop
   optimisation) would produce a `new_pc` with no corresponding `old_pc`,
   making the panic reachable.
2. Fuzz harnesses (RES-256) that feed arbitrary bytecode through the optimizer
   would trigger this if the invariant is violated.

The same class of defect was fixed in the compiler (`patch_jump`) by RES-263.

## Affected code

`resilient/src/peephole.rs` — `fn optimize`, jump-relinking loop (~line 119).

## Acceptance criteria

- An `OptimizeError` enum (or reuse / extend an existing error type) is added:
  ```rust
  #[derive(Debug)]
  enum OptimizeError {
      InternalError(&'static str),
  }
  ```
  (A unit struct with a message is also acceptable if the error is only ever
  propagated as a string.)
- `optimize` (currently `pub fn optimize(chunk: &mut Chunk)`) is changed to
  return `Result<(), OptimizeError>` (or equivalent).
- The `.expect(...)` on line 130 is replaced with:
  ```rust
  let Some(old_pc) = (0..chunk.code.len()).find(|&p| old_to_new[p] == new_pc) else {
      return Err(OptimizeError::InternalError("peephole: new_pc with no originating old_pc"));
  };
  ```
- All callers of `optimize` in `bytecode.rs` and elsewhere propagate the
  `Result` (the existing call site at `bytecode.rs` `Chunk::optimize` is
  the only one today).
- New unit test (in `peephole.rs`'s `#[cfg(test)]`):
  `optimize_returns_ok_for_normal_chunk` — run `optimize` on a basic
  chunk and assert `Ok(())` is returned.
- `cargo test` passes with 0 failures.
- `cargo clippy --all-targets -- -D warnings` clean.
- Commit: `RES-270: peephole optimize — replace expect() with Result return`.

## Notes

- Same class of defect as RES-263 (which converted `patch_jump`'s `panic!`
  to a `CompileError`). The same pattern applies here.
- The `old_to_new` map is built correctly today; the `expect` is a defensive
  assertion, not an infallible invariant. Returning an error is strictly safer
  than panicking for embedded targets.
- If `optimize` is called from `Chunk::optimize` in `bytecode.rs`, update that
  wrapper's signature too.

## Log

- 2026-04-20 created by analyzer (`peephole.rs` line 130 contains a production
  `.expect()` in the jump-relinking loop of the optimizer; same class as
  RES-263 which was fixed in `patch_jump`)
