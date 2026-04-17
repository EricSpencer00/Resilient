---
id: RES-198
title: `resilient lint` subcommand with 5 starter lints
state: OPEN
priority: P3
goalpost: tooling
created: 2026-04-17
owner: executor
---

## Summary
A lint subcommand is the vehicle for enforcing style beyond
formatter scope and catching common classes of bug that aren't
type errors. Start small — five lints, each with a stable code
and an `#[allow(...)]`-style suppress syntax.

## Acceptance criteria
- Subcommand `resilient lint <file>` runs the parser + typechecker
  + linter and prints per-lint diagnostics.
- Initial lints (each with a stable code `L0001`..`L0005`):
  - L0001: unused local binding
  - L0002: unreachable arm after `_ =>`
  - L0003: comparison `x == x` always true (typo smell)
  - L0004: mixing `&&` and `||` without parens
  - L0005: `return` at end of function body (redundant)
- Suppress syntax: `// resilient: allow L0003` on the line above
  the offending node.
- Exit 0 if no diagnostics; 1 if any lint fires at warning
  severity; 2 if any at error severity (none of the starter five
  are error-severity).
- Unit tests per lint: one triggering case + one allow-suppressed
  case.
- Commit message: `RES-198: resilient lint with 5 starter lints`.

## Notes
- Sharing infrastructure with `Diagnostic` (RES-119) lets lints
  go through the same LSP publish path "for free" — exposed in a
  follow-up.
- Lints are warnings, not errors, unless the user escalates via
  `--deny L0001` (pattern borrowed from rustc). Implement that
  flag in this ticket.

## Log
- 2026-04-17 created by manager
