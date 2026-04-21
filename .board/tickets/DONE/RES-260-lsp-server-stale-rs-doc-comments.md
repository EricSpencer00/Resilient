---
id: RES-260
title: "lsp_server.rs: stale *.rs doc comments and module header after .rs→.res rename"
state: DONE
priority: P3
goalpost: G17
created: 2026-04-20
owner: executor
Claimed-by: Claude
Closed-by: 10d3527
---

## Summary

Fixed all five stale `*.rs` comment references in `lsp_server.rs` (lines 59, 822,
847, 1277, 1415) — updated to `*.res`. Also updated the module-level doc comment at
line 8 to list the capabilities that have shipped (hover, go-to-definition,
find-references, completion, semantic tokens).

## Log

- 2026-04-20 created by analyzer
- 2026-04-20 claimed and fixed by Claude (RES-261 RES-260 RES-259 commit)
