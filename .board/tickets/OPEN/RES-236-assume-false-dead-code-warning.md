---
id: RES-236
title: "RES-133c: warn when `assume(false)` makes subsequent code unreachable"
state: OPEN
priority: P3
goalpost: G9
created: 2026-04-20
owner: executor
---

## Summary

`assume(false)` is a footgun: it silently passes all subsequent SMT
obligations (because `false` implies anything) and halts unconditionally
at runtime. The original RES-133 ticket specified a dead-code warning for
this case; this was explicitly deferred as RES-133c in the RES-133a
commit message.

A lint or typechecker warning at the `assume(false)` site prevents
accidental suppression of all verification in a function body.

## Acceptance criteria

- When the condition of `Node::Assume` is a `Node::BooleanLiteral { value: false, .. }`,
  emit a compiler diagnostic warning (not an error):
  `"assume(false): all subsequent verification obligations in this block
  are vacuously discharged; code after this point is unreachable at runtime"`.
- The warning is emitted during the typechecking or linting phase (not
  at parse time), so it respects the normal `--no-warn` suppression path
  if one exists.
- Unit test: source `assume(false);` in isolation produces the warning.
- Unit test: source `assume(true);` does NOT produce the warning.
- Unit test: source `assume(x > 0);` with a non-literal condition does
  NOT produce the warning (only literal `false` triggers it).
- `cargo test` remains fully green.
- `cargo clippy --all-targets -- -D warnings` remains clean.
- Commit message: `RES-236: warn on assume(false) dead-code region`.

## Notes

- This is a lint-level warning, not a hard error — some users may
  intentionally write `assume(false)` as a temporary stub, similar to
  `todo!()`.
- Do **not** modify existing tests — add only new ones.
- Consider adding the check to `resilient/src/lint.rs` as a new lint
  code (e.g. `L0005`) or to the typechecker; either location is
  acceptable. Document the choice in the PR description.

## Dependencies

- RES-133a (shipped in commit 6ada8e3) — `Node::Assume` exists in the AST.
- RES-235 (verifier threading) is a sibling, not a blocker — the warning
  is useful regardless of whether verifier threading has landed.

## Log
- 2026-04-20 created by analyzer (follow-up from RES-133a commit note)
