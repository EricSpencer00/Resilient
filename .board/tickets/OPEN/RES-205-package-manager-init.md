---
id: RES-205
title: `resilient pkg init` creates a minimal project skeleton
state: OPEN
priority: P3
goalpost: ecosystem
created: 2026-04-17
owner: executor
---

## Summary
One-liner onboarding: `resilient pkg init my-proj` creates a
directory with a manifest, a hello-world entrypoint, and a
gitignore. Doesn't yet imply a full package system — `init` is
the cheapest useful subcommand and lets us validate the manifest
schema before we build dependency resolution.

## Acceptance criteria
- Subcommand `resilient pkg init <name>`:
  - Creates `<name>/`.
  - Writes `<name>/Resilient.toml`:
    ```toml
    [package]
    name = "<name>"
    version = "0.1.0"
    edition = "2026-04"
    ```
  - Writes `<name>/src/main.rs` with a fn main that prints
    "Hello, world!".
  - Writes `<name>/.gitignore` ignoring `target/` and `cert/`.
- `resilient pkg init <existing-nonempty-dir>` errors without
  writing anything.
- Unit test: exec the subcommand in a tempdir, assert file
  presence + content.
- Commit message: `RES-205: pkg init subcommand`.

## Notes
- Don't implement `pkg add` / `pkg build` / dep resolution in
  this ticket — each deserves its own design pass.
- `edition` gives us a migration vector for future breaking
  changes. Start with an edition string tied to today's date for
  now.

## Log
- 2026-04-17 created by manager
