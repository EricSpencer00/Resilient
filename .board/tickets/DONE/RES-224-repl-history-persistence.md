---
id: RES-224
title: REPL history persistence — save/load ~/.resilient_history
state: DONE
priority: P3
goalpost: G11
created: 2026-04-20
owner: executor
Claimed-by: Claude
Closed-by: ed25d70
---

## Summary
The REPL loses all input history on exit. Wire up `rustyline`'s history API to persist history to `~/.resilient_history` across sessions.

## Acceptance criteria
- On REPL startup, load history from `~/.resilient_history` if the file exists. Silently skip if missing or unreadable.
- On clean exit (Ctrl-D / `exit`), save history to the same file. A save failure emits a warning to stderr but does not crash.
- History file location overridable via `RESILIENT_HISTORY` environment variable.
- History limit is 1000 entries (compile-time constant in `repl.rs`).
- `~` expansion done explicitly via `std::env::var("HOME")`.
- No new dependencies needed (rustyline already present).
- Commit message: `RES-224: persist REPL history to ~/.resilient_history`.

## Notes
- `rustyline::Editor::load_history` and `save_history` are the relevant calls.

## Log
- 2026-04-20 created by manager
</content>
</invoke>