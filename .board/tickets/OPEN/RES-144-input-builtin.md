---
id: RES-144
title: `input(prompt) -> String` builtin reads a line from stdin
state: OPEN
priority: P3
goalpost: G11
created: 2026-04-17
owner: executor
---

## Summary
Teaching examples need interactivity — "enter your name" style. A
single-line read from stdin, optional prompt printed first, is the
minimal version.

## Acceptance criteria
- Builtin `input(prompt: String) -> String`. `input("")` is the
  prompt-less form.
- Reads up to the first `\n`, strips trailing `\r\n`/`\n`.
- EOF before any data returns `""`.
- Only in std build; the no_std runtime doesn't get this.
- Unit test uses a stubbed stdin (via
  `std::io::Cursor` injected through a small trait) to verify
  behavior without blocking the test on real stdin.
- `examples/interactive_greeter.rs` — `cargo run` demo; no golden
  (it's interactive).
- Commit message: `RES-144: input() builtin`.

## Notes
- Don't read from stdin at build time — the golden-test framework
  walks examples/ and runs them; this example must be skipped by
  the harness. Extend the harness to look for an `.interactive`
  sidecar file that marks an example as "don't exec in CI".
- Returning `""` on EOF rather than erroring is deliberate: lets
  loops like `while (input("> ") != "quit")` terminate on
  ctrl-D.

## Log
- 2026-04-17 created by manager
