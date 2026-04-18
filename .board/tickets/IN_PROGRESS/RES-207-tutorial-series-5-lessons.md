---
id: RES-207
title: Tutorial series — 5 lesson pages on the docs site
state: IN_PROGRESS
priority: P3
goalpost: docs
created: 2026-04-17
owner: executor
---

## Summary
The docs site (Jekyll) has a getting-started page. The next
onboarding step is a guided tutorial. Five lessons, each
self-contained and building on the previous, take a new user
from "hello" to "verified live block". Pairs with RES-206's
error index as the two main docs deliverables.

## Acceptance criteria
- `docs/tutorial/` contains:
  - `01-hello.md` — installation, REPL, running an example.
  - `02-variables-and-types.md` — let, primitives, static typing.
  - `03-functions-and-contracts.md` — fn, requires/ensures,
    running `--audit`.
  - `04-live-blocks.md` — writing a self-healing div-by-zero
    example, `live_retries()`, telemetry.
  - `05-verifying-with-z3.md` — building with --features z3,
    emitting a certificate, verifying it independently.
- Each page ends with a "what's next" link.
- Every code snippet copy-pastes into an actual runnable program
  — verified by a companion script `docs/verify_tutorial_snippets.sh`
  that greps the code blocks and runs each through the CLI.
- Nav updates on the site to surface the tutorial section.
- Commit message: `RES-207: 5-lesson tutorial series`.

## Notes
- Length target per lesson: 400-700 words + a few code blocks.
  Longer pages lose readers.
- Lesson 5 may need to gate behind the `z3` feature in the
  snippet-verification step; document the prerequisite in the
  lesson intro.

## Log
- 2026-04-17 created by manager
- 2026-04-17 claimed by executor
