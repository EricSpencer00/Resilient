---
id: RES-206
title: Error-code registry (E0001..E00NN) + docs page per code
state: IN_PROGRESS
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
- 2026-04-17 claimed and bailed by executor (blocked on RES-119)
- 2026-04-17 claimed by executor — landing RES-206a scope (registry module +
  initial ~10 codes + sample docs pages + nav entry) now that RES-119 delivered
  `DiagCode` + `Diagnostic`

## Attempt 1 failed

Blocked on RES-119. The ticket's opening sentence — "RES-119
introduced a `DiagCode` newtype with no populated registry.
Populate it." — presupposes `diag::DiagCode` exists. RES-119 is
currently in OPEN with a `## Clarification needed` note (an
internal scope conflict the Manager needs to resolve), so neither
`resilient/src/diag.rs` nor `DiagCode` exists on `main` today.

Every acceptance criterion in this ticket references `DiagCode`:

- "A central registry `resilient/src/diag/codes.rs`: `pub const
  E0001: DiagCode = DiagCode("E0001");`" — needs `DiagCode`.
- "Every existing diagnostic assigned a code ... Diagnostic
  rendering shows the code inline" — needs both the registry and
  the `Diagnostic` struct RES-119 defines to carry the code field.

Even the docs half (40 hand-written `.md` pages under `docs/errors/`
+ website nav) only has value once the source emits the codes
inline.

## Clarification needed

Gate this ticket on RES-119 (or on whichever rewrite of it the
Manager chooses — see RES-119's `## Clarification needed`). Once
the `Diagnostic` scaffolding lands, RES-206 is self-contained:
audit every error-creation site, assign codes, render inline,
write ~40 docs pages.

No code changes landed — only the ticket state toggle and this
clarification note.
