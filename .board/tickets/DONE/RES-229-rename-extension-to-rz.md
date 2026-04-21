---
id: RES-229
title: Rename source file extension from `.rl` to `.rz`
state: DONE
priority: P2
goalpost: G14
created: 2026-04-21
closed: 2026-04-21
owner: executor
claimed-by: Claude
---

## Summary
Change the canonical Resilient source file extension from `.rl` to `.rz`.
Update the compiler CLI, REPL, imports resolver, package init, LSP server,
VS Code extension, test fixtures, golden files, docs, and issue templates.

## Acceptance criteria
- All `resilient/examples/*.rl` files renamed to `*.rz` (golden
  `.expected.txt` sidecars keep their paired basenames).
- `resilient run`, `resilient check`, `resilient compile` accept `.rz`
  as the canonical extension. `.rl` no longer referenced in CLI error
  messages, help text, or usage examples.
- Import resolver (`resilient/src/imports.rs`) resolves module imports
  against `.rz` files.
- `resilient init` / package scaffolder emits `main.rz` rather than
  `main.rl`.
- VS Code extension (`vscode-extension/package.json`) registers `.rz`
  for the Resilient language.
- LSP server associates the Resilient language ID with `.rz`.
- All docs (`docs/syntax.md`, `docs/getting-started.md`,
  `docs/community.md`, plan files under `docs/superpowers/plans/`) refer
  to `.rz`.
- Issue templates (`.github/ISSUE_TEMPLATE/bug-report.yml`) refer to
  `.rz`.
- `cargo fmt`, `cargo clippy --all-targets -- -D warnings`, and
  `cargo test` all pass.
- Commit message: `RES-229: rename source file extension to \`.rz\``.

## Notes
- Do not keep `.rl` as a legacy alias — this is a clean rename.
- Leave `.worktrees/` untouched; those are other agents' workspaces.
- Test fixtures under `resilient/tests/` that embed filenames in asserts
  must be updated to the new extension.

## Log
- 2026-04-21 created and claimed by Claude
- 2026-04-21 implemented end-to-end; build + tests + clippy clean; moved to DONE
