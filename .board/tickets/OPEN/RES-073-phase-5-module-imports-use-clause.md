---
id: RES-073
title: Phase 5 module imports use clause
state: OPEN
priority: P2
goalpost: G15
created: 2026-04-17
owner: executor
---

## Summary
Resilient currently runs a single `.res` file with no way to bring in
declarations from another file. To make stdlib growth (and any non-trivial
program) survivable we need a `use` clause that imports the public
functions of one file into another. This is the minimum-viable module
system: file path → namespace, all top-level `fn` definitions exported,
no submodules, no visibility modifiers yet.

## Acceptance criteria
- New top-level statement: `use "path/to/other.res";` (string is a path
  relative to the importing file's directory).
- Imported file's top-level functions become callable from the importer.
  Plain identifier (`other_fn(x)`) is enough — no qualified `module::other_fn`
  syntax in this ticket.
- Cycles produce a clean diagnostic, not a stack overflow.
- Re-importing the same file from two places parses-and-loads it once.
- New example `resilient/examples/imports_demo/main.res` that imports
  `helpers.res` and calls one of its functions; pinned by a golden test
  in `resilient/tests/examples_golden.rs`.
- Lexer change: add a `Use` keyword token.
- Parser change: `parse_use_statement` returns a new `Node::Use { path: String, span: Span }` variant.
- Interpreter pre-pass walks `Use` nodes, loads each file, parses it,
  prepends its top-level `Function` nodes to the current `Program`.
- `cargo build`, `cargo test`, `cargo clippy -- -D warnings` all pass.
- Commit message: `RES-073: use "path.res"; imports top-level fns`.

## Notes
- Lexer + parser live in `resilient/src/main.rs`.
- For path resolution use `std::path::Path::join` against the importing
  file's parent directory. Bail with a clean error if the file doesn't
  exist.
- Span field requires RES-069 to land first if you want fully-spanned
  diagnostics — otherwise you can plumb a synthetic span and follow up.

## Log
- 2026-04-17 created by manager
- 2026-04-17 acceptance criteria filled in by manager
