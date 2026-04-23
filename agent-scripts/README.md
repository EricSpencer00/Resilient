# agent-scripts

Tooling for autonomous agent dispatch on Resilient. Layers on top of the
file-claims + extension-point system introduced in PR #230.

## Scripts

| Script | Purpose |
|---|---|
| `check-overlaps.sh` | Pre-dispatch: verify files don't conflict with open PRs or active claims |
| `claim-files.sh` | Register files owned by a branch |
| `release-claims.sh` | Release claims (called by CI on PR merge) |
| **`pick-ticket.sh`** | Select the next `agent-ready` ticket not already in an open PR |
| **`dispatch-agent.sh`** | End-to-end: pick ticket, create worktree + branch, open draft PR |
| **`agent-status.sh`** | One-screen view of worktrees, open PRs, claims, and the next ticket |

## Typical autonomous loop

```bash
# 1. See what's in flight
agent-scripts/agent-status.sh

# 2. Dispatch a worker on the next agent-ready ticket
agent-scripts/dispatch-agent.sh           # or: --issue 167
# → creates .claude/worktrees/res-167/, branch res-167-<slug>, draft PR

# 3. In another shell or an agent session
cd .claude/worktrees/res-167
agent-scripts/claim-files.sh res-167-<slug> resilient/src/main.rs ...
# ... implement, commit, push — PR auto-updates
gh pr ready <pr-url>
```

## Picker rules

`pick-ticket.sh` excludes an issue if:

- It isn't labeled `agent-ready`, or it's closed.
- An open PR references it — either `Closes #N` in the body, a matching
  `RES-NNN` in the title/body/branch, or a `res-NNN-*` branch name.
- It's assigned to a human (non-bot). The Copilot and Claude bot
  accounts are treated as non-human and don't block the pick.

## Dispatch rules

`dispatch-agent.sh`:

- Always branches off `origin/main`, not off any in-flight branch.
- Creates the worktree at `.claude/worktrees/res-<N>/`.
- Opens a draft PR with `Closes #<N>` so `pick-ticket.sh` sees it as
  claimed on the next run. An empty claim commit lets `gh pr create`
  compute a diff.
- Does **not** run the agent itself. The caller (a human, another
  script, or a spawned Claude agent) does the actual work inside the
  worktree.

## Why not GitHub Projects?

GitHub Projects requires `read:project` / `project` OAuth scopes that
the default `gh auth` token usually lacks. Issues with the
`agent-ready` label are a functional substitute — they already
describe a "todo" queue, `gh issue list` is fast, and there's no
second source of truth to keep in sync. If a proper Kanban view
becomes necessary, wire it up as a read-only projection from
`pick-ticket.sh --json` rather than moving state into Projects.
