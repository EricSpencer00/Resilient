---
id: RES-247
title: "LSP: stale *.rs doc comments in lsp_server.rs after .rs→.res rename"
state: DONE
priority: P3
goalpost: G17
created: 2026-04-20
owner: executor
claimed-by: Claude
closed-by: 561212ddecd69636b3a5357d0875265ec0706e19
---

## Summary

Two doc comments in `resilient/src/lsp_server.rs` still reference `*.rs`
after the workspace walker was updated to match `*.res` files. They are
cosmetically wrong and will mislead contributors.

## Affected locations

| Line | Current text | Should read |
|------|-------------|-------------|
| 625 | `/// RES-186: recursive \`*.rs\` walker. Skips \`target/\` and any` | `/// RES-186: recursive \`*.res\` walker. Skips \`target/\` and any` |
| 1167 | `/// index on first call (walks the workspace root for \`*.rs\`` | `/// index on first call (walks the workspace root for \`*.res\`` |

The production code at line 643 already uses `Some("res")` — only the
comments are out of date.

## Acceptance criteria

- Update both doc comments so they say `*.res` (not `*.rs`).
- `cargo test --features lsp` remains green.
- `cargo clippy --all-targets --features lsp -- -D warnings` remains clean.
- Commit message: `RES-247: fix stale *.rs doc comments in lsp_server.rs after rename`.

## Notes

- Doc-comment-only change; no logic or tests need to change.
- Do not alter any test code or the production `walk_resilient_files`
  implementation.

## Log
- 2026-04-20 created by analyzer (stale *.rs doc comments found in lsp_server.rs)
- 2026-04-20 claimed by Claude, fixed all stale `*.rs` doc comments (4 occurrences)
