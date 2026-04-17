---
id: RES-206
title: Error-code registry (E0001..E00NN) + docs page per code
state: OPEN
priority: P2
goalpost: docs
created: 2026-04-17
owner: executor
---

## Summary
RES-119 introduced a `DiagCode` newtype with no populated
registry. Populate it. Every diagnostic the compiler can emit
gets a stable code and a page on the docs site explaining the
cause, a minimal reproducing example, and the standard fix.

## Acceptance criteria
- A central registry `resilient/src/diag/codes.rs`:
  ```rust
  pub const E0001: DiagCode = DiagCode("E0001"); // ...
  ```
- Every existing diagnostic assigned a code (at least the
  ~40 distinct ones currently emitted — audit the codebase).
- Diagnostic rendering shows the code inline:
  `foo.rs:3:5: error[E0007]: expected `;``.
- `docs/errors/E0007.md` (Jekyll page) per code with: headline,
  what triggers it, a 4-line minimal example, the fix, a link
  back to the source tree line that emits it.
- Website's nav gains an "Error index" entry.
- Commit message: `RES-206: error-code registry + docs pages`.

## Notes
- Docs generation: don't automate page creation from the registry
  yet — hand-write the ~40 pages to ensure quality. Automation
  is a follow-up once the baseline exists.
- Code numbers are sticky. Once assigned, never reuse.

## Log
- 2026-04-17 created by manager
