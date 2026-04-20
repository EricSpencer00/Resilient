---
id: RES-245
title: "Add assume_demo.res + .expected.txt golden sidecar for assume() feature"
state: OPEN
priority: P3
goalpost: testing
created: 2026-04-20
owner: executor
---

## Summary

`assume(expr)` was shipped in RES-133a with unit tests inside `#[cfg(test)]`
but without a `resilient/examples/assume_demo.res` example or its
`.expected.txt` golden sidecar. CLAUDE.md requires:

> New language features: add an `.expected.txt` golden sidecar in
> `resilient/examples/`.

The existing `bind_pattern_demo.res` (added by RES-234) is the reference
pattern to follow.

## What to add

**`resilient/examples/assume_demo.res`** — a short runnable program that
demonstrates the three core uses of `assume()`:

1. `assume(cond)` — unconditional: passes silently when `cond` is true.
2. `assume(cond, msg)` — with custom message.
3. `assume(true)` followed by a computation that uses the established fact
   to show assume composes with subsequent code.

The file should use `println` to produce observable output so the golden
file has something meaningful to match.

**`resilient/examples/assume_demo.expected.txt`** — the expected stdout
output when `resilient examples/assume_demo.res` runs.

## Acceptance criteria

- `resilient/examples/assume_demo.res` exists and is syntactically valid.
- `resilient/examples/assume_demo.expected.txt` exists and matches the
  actual output of `resilient examples/assume_demo.res`.
- The `golden_outputs_match` test in `resilient/tests/examples_golden.rs`
  passes with the new sidecar included.
- `cargo test` remains fully green.
- `cargo clippy --all-targets -- -D warnings` remains clean.
- Commit message: `RES-245: add assume_demo golden sidecar example`.

## Reference

- `resilient/examples/bind_pattern_demo.res` — existing demo to follow as
  a style template.
- `resilient/examples/bind_pattern_demo.expected.txt` — corresponding golden.
- RES-133a (commit `6ada8e3`) — the PR that shipped `assume()`.
- RES-234 (DONE) — the ticket that added the bind-pattern golden sidecar,
  establishes the precedent.

## Notes

- Do **not** demonstrate `assume(false)` in the example — that would abort
  execution. Use only cases where the condition is true at runtime.
- Do **not** rely on the SMT/Z3 verifier path (RES-235) — the demo should
  work with the default tree-walking interpreter (`resilient assume_demo.res`).

## Log

- 2026-04-20 created by analyzer (RES-133a shipped without golden sidecar)
