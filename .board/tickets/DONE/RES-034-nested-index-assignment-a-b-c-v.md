---
id: RES-034
title: Nested index assignment a[b][c] = v
state: DONE
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
- 2026-04-17 executor landed:
  - Interpreter `Node::IndexAssignment` arm rewritten: walks the LHS
    chain (a series of nested `IndexExpression`s) collecting every
    index expr until it hits the root `Identifier`. Original
    "must be an identifier" error preserved as the fallback for non-
    `IndexExpression` LHS shapes (e.g. `(some + expr)[i] = v`).
  - New free function `replace_at_path(&mut [Value], &[i64], Value)
    -> RResult<()>` recurses through the array tree using
    `std::mem::replace` to avoid cloning the inner Vec on each level.
    Bounds errors include `at dim {N}` so users can tell outer-vs-
    inner failures apart.
  - Evaluation order preserved: RHS evaluated first, then index exprs
    in source order (root-to-leaf), then read-modify-write.
- 2026-04-17 tests: 6 new unit tests in `main.rs` mod tests:
  - `nested_index_assignment_writes_leaf_cell`: 2D mutation
  - `nested_index_assignment_leaves_siblings_untouched`: confirms
    only the addressed cell changes (regression guard against
    accidental aliasing)
  - `nested_index_assignment_outer_out_of_bounds_errors_cleanly`
    (mentions "dim 1")
  - `nested_index_assignment_inner_out_of_bounds_errors_cleanly`
    (mentions "dim 2")
  - `three_deep_nested_index_assignment`: arbitrary depth
  - `single_dim_index_assignment_still_works`: 1D regression guard
  - Plus `examples/nested_array_demo.rs` + `.expected.txt` golden
    proving 2x3 matrix mutation end-to-end.
- 2026-04-17 verification: 165 unit + 1 golden + 6 smoke = 172 tests
  default; 173+1+7 = 181 with `--features z3`. `cargo build`,
  `cargo clippy -- -D warnings` clean both ways. Manual run of the
  demo file prints `1\n2\n3\n4\n99\n6` as expected.
