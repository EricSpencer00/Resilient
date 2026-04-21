---
id: RES-283
title: "Board hygiene: fixes for RES-262/263/264/265/266/267/268/269 are on worktree branches, not on main"
state: OPEN
priority: P1
goalpost: G3
created: 2026-04-20
owner: executor
---

## Summary

Nine worktree branches each contain one or more committed fixes for
DONE-marked tickets whose code has **never been merged to main**. The
`main` branch therefore still has the production bugs those tickets
describe, even though the board shows them as closed.

## Affected tickets and worktree branches

| Ticket | Subject | Worktree branch | Commit |
|---|---|---|---|
| RES-262 | walk_call_sites missing AssumeStatement/MapLiteral/etc | worktree-agent-a1e5beff | 83c4869 |
| RES-263 | patch_jump panics on non-jump op | worktree-agent-a1e5beff | 83c4869 |
| RES-264 | builtin_format `rest.chars().next().unwrap()` | worktree-agent-a1e5beff | 83c4869 |
| RES-265 | walk_call_sites misses ImplBlock methods | worktree-agent-a72b67d1 | 04d9668 |
| RES-266 | LSP ImplBlock invisible to symbols/completion/rename | worktree-agent-a72b67d1 | 04d9668 |
| RES-267 | lint recurse_children misses MapLiteral/SetLiteral/LetDestructureStruct | worktree-agent-a8881c47 | a32ed1e |
| RES-268 | walk_call_hints misses most AST node variants | worktree-agent-a72b67d1 | 04d9668 |
| RES-269 | JIT error unsupported-static-str loses callee name | worktree-agent-a8881c47 | a32ed1e |

In addition, the following branches contain fixes for still-open tickets:

| Ticket | Subject | Worktree branch | Commit |
|---|---|---|---|
| RES-259 | L0001 does not fire for unused match-arm bindings | worktree-agent-aae05f42 | 10d3527 |
| RES-260 | lsp_server.rs stale *.rs doc comments | worktree-agent-aae05f42 | 10d3527 |
| RES-243 | imports test flaky: shared fixed temp-file path | worktree-agent-abd834f9 | 8140f38 |

## Root cause

Each worktree agent commits its fix to a local worktree branch and closes
the ticket on the board, but the worktree branch is never opened as a PR
or merged back to `main`. The board state diverges from the main branch.

## Verification (checked 2026-04-20)

```
# On main these bugs are still present:
# RES-263: bytecode.rs patch_jump still has panic!("patch_jump called on non-jump op")
# RES-264: main.rs builtin_format still has rest.chars().next().unwrap()
# RES-265/266/268: lsp_server.rs has no ImplBlock handling in walk_call_sites/walk_call_hints
# RES-267: lint.rs recurse_children lacks MapLiteral/SetLiteral/LetDestructureStruct arms
# RES-269: jit_backend.rs unsupported error loses callee name
```

## Acceptance criteria

- For tickets RES-262 through RES-269: for each affected worktree branch,
  either:
  - Cherry-pick or rebase the fix commit onto main and open a PR, OR
  - Re-implement the fix directly on a new branch from main.
- Each fix must pass `cargo test` and `cargo clippy --all-targets -- -D warnings`.
- Ticket files for RES-262 through RES-269 (currently in `DONE/` with
  `state: OPEN` or `state: DONE` but un-merged code) must be updated to
  reflect the actual merged commit hash.
- RES-259 and RES-260 fixes can be included in the same sweep.
- NOTE: 10d3527 (worktree-agent-aae05f42) also touches test files; the PR
  must include a "Test changes" section per CLAUDE.md test-protection policy.
- Commit format: `RES-283: merge worktree fixes for RES-262..269 to main`.

## Notes

- The RES-230 worktree fix (worktree-agent-ab670c73 / 7c1ffff) is a very
  large refactor (3912 insertions, 37 files including many test files) and
  touches the bulk of the compiler. It requires maintainer approval before
  merge. Handle separately.
- Prefer separate PRs per worktree to keep review scope manageable.
- Do NOT amend or force-push existing worktree branches; create new branches
  from main instead.

## Log

- 2026-04-20 created by analyzer (cargo test and clippy clean on main;
  cross-referencing DONE ticket states against main branch code confirmed
  9 worktree branches with unmerged fixes)
