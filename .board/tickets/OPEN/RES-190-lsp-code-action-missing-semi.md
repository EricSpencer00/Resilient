---
id: RES-190
title: LSP: code action "insert `;`" for missing-semicolon diagnostics
state: OPEN
priority: P3
goalpost: G17
created: 2026-04-17
owner: executor
---

## Summary
First tangible code action — give users the "quick fix" lightbulb
experience for the single most common parser error. Lays the
pattern that other actions (delete unreachable arm, add `_`, etc.)
can follow.

## Acceptance criteria
- `Backend::code_action` returns a `CodeAction` when the diagnostic
  at the requested range has code "E-missing-semicolon"
  (introduced in RES-206's registry).
- The action's `WorkspaceEdit` inserts `;` at the end of the
  preceding token.
- Integration test opens a document with a missing `;`, requests
  code actions at the diagnostic range, asserts the action is
  present and its edit lines up with a fixed version.
- Commit message: `RES-190: LSP code action: insert missing semicolon`.

## Notes
- Depends on RES-119's Diagnostic carrying a stable code. If
  RES-206 hasn't registered "E-missing-semicolon" yet, wire a
  placeholder code and backfill when RES-206 lands.
- Don't auto-apply — editor UX presents the action and the user
  confirms.

## Log
- 2026-04-17 created by manager
