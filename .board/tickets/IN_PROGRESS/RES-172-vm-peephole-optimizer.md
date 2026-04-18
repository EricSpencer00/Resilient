---
id: RES-172
title: VM: peephole optimizer pass for common op sequences
state: IN_PROGRESS
priority: P3
goalpost: G15
created: 2026-04-17
owner: executor
---

## Summary
The VM emits naive bytecode: `LoadConst 0; Add` instead of the
no-op, `LoadLocal x; LoadConst 1; Add; StoreLocal x;` instead of
an (absent) `IncLocal`. A peephole pass running after compilation
catches the worst of these. Targets: add 10-15% on fib-style
benches for free.

## Acceptance criteria
- New module `bytecode/peephole.rs` with a `optimize(&mut Chunk)`
  entry point invoked after RES-076's compiler emits code.
- Rules to ship in this ticket:
  - `LoadConst 0; Add` → drop both (identity).
  - `LoadConst 1; Add; StoreLocal x` (if preceded by `LoadLocal x`)
    → `IncLocal x` (new opcode if not present).
  - `Jump to next instruction` → drop.
  - `Not; JumpIfFalse` → `JumpIfTrue`.
- Each rule behind a predicate so we can unit-test them
  individually.
- Relink `Jump` / `JumpIfFalse` offsets after every transformation
  via a fixup table (don't hand-compute).
- Bench RESULTS.md updated: fib(25) speedup after peephole.
- Commit message: `RES-172: VM peephole optimizer pass`.

## Notes
- Keep the pass O(n·k) where k is small — single linear scan with
  a small window. Don't do iterative fixed-point yet.
- Line-info preservation: the peephole must keep `chunk.line_info`
  consistent (RES-091) — tests explicitly assert a runtime error
  after optimization still prints the correct source line.

## Log
- 2026-04-17 created by manager
- 2026-04-17 claimed by executor
