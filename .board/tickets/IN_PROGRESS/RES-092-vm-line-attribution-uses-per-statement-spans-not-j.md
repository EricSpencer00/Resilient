---
id: RES-092
title: VM line attribution uses per-statement spans (refines RES-091)
state: OPEN
priority: P2
goalpost: G15
created: 2026-04-17
owner: executor
---

## Summary
RES-091 wired VM runtime errors to print `(line N)`, but the
attribution lands too coarse: the entire fn body shows the same
line because `compiler::compile` only takes the OUTER spanned
statement's line and passes it to every `chunk.emit(op, line)` call
inside that function.

This ticket refines compile.rs to thread each fn-body statement's
**own** span line through to the inner `chunk.emit` calls. After
this, a divide-by-zero on line 5 of a 10-line function reports
`(line 5)`, not `(line 1)`.

The body statements in a fn are bare `Node`s (not `Spanned<Node>`),
but they ARE struct variants with their own `span` fields after the
G6 work (RES-079, RES-085, RES-087). The compiler just needs to
read those spans.

## Acceptance criteria
- `compiler::compile_stmt_in_fn` (and the related
  `compile_control_flow_in_fn`) computes the line for `chunk.emit`
  from the **statement node's own span**, not the parameter passed
  in from the caller.
- A small `node_line` helper in compiler.rs extracts a u32 line from
  any Node variant by reaching into its span field — falls back to
  the caller-supplied default when the node has no span (or a
  default span). Use a match over the variants that have spans
  (statements + leaves + control-flow).
- New unit test: compile-and-run a fn whose body has a divide-by-
  zero on line 3 of source. Assert the error's `Display` contains
  `line 3` (or `line 4` depending on `\n` handling — the goal is
  NOT line 1).
- Existing tests still pass — the only difference users see is
  more precise `(line N)` suffixes.
- `cargo build`, `cargo test`, `cargo clippy -- -D warnings` pass
  on all three feature configs.
- Commit message: `RES-092: VM line info threads per-statement spans (refines RES-091)`.

## Notes
- compiler.rs is at `resilient/src/compiler.rs`. Look for the
  `let line = spanned.span.start.line as u32;` pattern in `compile`
  — that's where the outer fn's line gets locked in. The body-
  statement loop should derive a fresh line per statement.
- Body statements are `&Node` (function decls store `body: Box<Node>`
  which is typically a `Node::Block { stmts, span }`). Walk the
  `stmts` and use each one's individual span line.
- The compile_expr helper takes `line: u32` — keep it; just pass
  the per-statement line into it.

## Log
- 2026-04-17 created by manager
- 2026-04-17 acceptance criteria filled in by manager (orchestrator pass)
