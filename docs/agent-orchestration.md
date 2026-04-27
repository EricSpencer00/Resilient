# Agent Orchestration Hardening

Resilient uses GitHub Issues and pull requests as the source of truth for
agent work. The local scripts provide scheduling and guardrails, but they
must not create a second canonical board.

## Control Plane

The orchestration loop has five phases:

1. **Pick** an `agent-ready` issue that has no open PR claim.
2. **Dispatch** a fresh branch and worktree from `origin/main`.
3. **Claim** expected files before the executor starts editing.
4. **Verify** with `verify-scope.sh` and CI before a PR leaves draft.
5. **Sync** through `agents/integration` before auto-merge.

`agent-scripts/agent-status.sh --json` is the dashboard feed. A Kanban UI
may be built on top of that JSON, but the UI must remain a projection of
issues, PRs, CI, worktrees, and file claims.

## Resumability

Agents should assume model context can disappear at any point. The durable
handoff is a PR comment emitted by `agent-handoff.sh` at dispatch,
executor start, guardrail failure, guardrail success, and orchestrator
finish. A replacement agent resumes from:

- the linked issue body,
- the PR body and latest handoff comments,
- branch commits and changed files,
- CI check state,
- the latest guardrail report.

## Conflict Policy

File claims are leases, not ownership forever. Dispatch refuses known
overlaps before work begins, `verify-scope.sh` checks overlap before a PR
is marked ready, and merge releases claims through
`release-file-claims.yml`.

Hard conflicts require human resolution. Append-only extension conflicts
may be handled by `sync-integration.sh` and `auto-resolve-extensions.sh`.

## Formal Model

The TLA+ model in `docs/agent-orchestration.tla` captures the safety
invariants this workflow is meant to preserve:

- no two live PRs own the same claimed file,
- a ready PR has passed guardrails,
- an auto-mergeable PR has passed guardrails and integration sync,
- merged PRs have released their file claims.
