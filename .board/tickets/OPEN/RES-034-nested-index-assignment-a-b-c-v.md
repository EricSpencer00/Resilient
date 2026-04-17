---
id: RES-034
title: Nested index assignment a[b][c] = v
state: OPEN
priority: P2
goalpost: G12
created: 2026-04-16
owner: executor
---

## Summary
The interpreter handles `a[i] = v` (single index) but rejects
`a[i][j] = v` with the error `Index assignment target must be an
identifier` (see `resilient/src/main.rs:2941`). Multi-dimensional
arrays are unusable for assignment as a result. This ticket lifts the
restriction by walking the LHS chain from the innermost index back to
the root identifier, then performing a read–modify–write on the
nested arrays. Field-access chains (`a.b.c = v`) and mixed
field/index chains (`a.b[i].c = v`) are out of scope here — separate
tickets.

## Acceptance criteria
- `a[i][j] = v` works for nested arrays of any depth (parser already
  produces the right shape; the fix is in the interpreter's
  `Node::IndexAssignment` arm).
- Error messages stay specific: out-of-bounds on the *innermost*
  index reports the offending dim, not just the outer.
- Read–modify–write semantics: only the path being mutated is rebuilt
  on the env; sibling cells in the outer arrays are untouched (no
  visible mutation of values bound to other names — current value
  semantics preserved).
- New unit tests in `main.rs` `#[cfg(test)] mod tests`:
  - `let m = [[1, 2], [3, 4]]; m[1][0] = 9; assert m[1][0] == 9;`
  - `let m = [[1, 2], [3, 4]]; m[0][1] = 9; assert m[0][0] == 1;` (sibling untouched)
  - `let m = [[1, 2], [3, 4]]; m[2][0] = 9;` → clean `Index 2 out of bounds for array of length 2` (outer-bounds case still works).
  - `let m = [[1, 2]]; m[0][5] = 9;` → clean inner-out-of-bounds error mentioning the inner dim.
- New `examples/nested_array_demo.rs` + `.expected.txt` golden building a 2×3 matrix and mutating one cell.
- `cargo build`, `cargo test`, `cargo clippy -- -D warnings` all pass.
- Commit message: `RES-034: nested index assignment a[i][j] = v`.

## Notes
- Parser shape: for `a[i][j] = v`, the LHS parses as
  `IndexExpression { target: IndexExpression { target: Identifier("a"), index: i }, index: j }`.
  The current interpreter (`main.rs:2937`) only matches when `target` is
  `Identifier`. The fix is to recursively descend `target`, collect the
  index chain, then unwind: read the outer array, dive in by each index
  except the last, mutate at the final index, and write the chain back
  up. Easiest implementation: recursive helper that takes `(env, &mut
  Value::Array, &[Value])` and replaces the leaf.
- Watch out for the existing `target must be an identifier` error message —
  keep it as the fallback for *non*-index LHS shapes (e.g.
  `(some + expr) = v`), don't widen it.
- Will likely need to refactor the read-modify-write to clone the
  outer Array first, mutate the clone, then `env.reassign`. The naked
  `Value::Array(mut items)` destructure today moves out of `current`,
  which is fine because `env.get` returns a clone — keep that property.

## Log
- 2026-04-16 created by manager
- 2026-04-17 acceptance criteria filled in by manager (orchestrator pass)
