---
id: RES-228
title: `resilient run --watch` — re-run on file save
state: OPEN
priority: P3
goalpost: G11
created: 2026-04-20
owner: executor
---

## Summary
Add a `--watch` flag to `resilient run` that watches the source file (and any `use`d imports) for changes and automatically re-runs the program on save.

## Acceptance criteria
- `resilient run --watch file.rs` runs immediately, then re-runs on each save.
- Output from each run is preceded by `--- [re-run at HH:MM:SS] ---`.
- `Ctrl-C` exits cleanly.
- All files `use`d by the source are also watched.
- Debounce of 200 ms prevents double-fires from editors that write in two steps.
- Works on macOS (FSEvents) and Linux (inotify) — use the `notify` crate (v6.x).
- Only one run in-flight at a time — if re-triggered, cancel previous run (SIGTERM child).
- `--watch` silently ignored when stdin is not a TTY (piped CI usage).
- Commit message: `RES-228: \`resilient run --watch\` re-runs on file save with debounce`.

## Notes
- `notify` crate v6.x has a stable cross-platform API; add it as a dependency.

## Log
- 2026-04-20 created by manager
</content>
</invoke>