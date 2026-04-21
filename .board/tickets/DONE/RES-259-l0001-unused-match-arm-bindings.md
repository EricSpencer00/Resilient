---
id: RES-259
title: "L0001: lint does not fire for unused match-arm bindings"
state: DONE
priority: P3
goalpost: G14
created: 2026-04-20
owner: executor
Claimed-by: Claude
Closed-by: TBD
---

## Summary

Extended `collect_lets_in` in `lint.rs`:
- Added `collect_pattern_bindings` helper that extracts `Pattern::Identifier(name)`
  and `Pattern::Bind(name, _)` names as let-equivalent declarations.
- Added `ExpressionStatement` and `ReturnStatement` descent to `collect_lets_in`
  so match expressions used as statement values are also walked.
- Extended the `Node::Match` arm handler to call `collect_pattern_bindings` for
  each arm's pattern.
- Added 4 new tests: fires on unused identifier binding, silent when used, silent
  for underscore-prefixed binding, fires for unused `name @ inner` bind pattern.

## Log

- 2026-04-20 created by analyzer
- 2026-04-20 claimed and fixed by Claude (RES-261 RES-260 RES-259 commit)
