# agent-scripts

Tooling for autonomous agent dispatch on Resilient. Layers on top of the
file-claims + extension-point system introduced in PR #230.

## Scripts

| Script | Purpose |
|---|---|
| `check-overlaps.sh` | Pre-dispatch: verify files don't conflict with open PRs or active claims |
| `claim-files.sh` | Register files owned by a branch |
| `release-claims.sh` | Release claims (called by CI on PR merge) |
| `pick-ticket.sh` | Select the next `agent-ready` ticket not already in an open PR |
| `dispatch-agent.sh` | Create worktree + branch + draft PR for a ticket |
| `agent-status.sh` | One-screen view of worktrees, open PRs, claims, and the next ticket |
| **`verify-scope.sh`** | Guardrail: diff-shape + fmt + clippy + test + overlap, writes JSON report |
| **`ready-or-bail.sh`** | Runs `verify-scope.sh`; marks PR ready on green, posts failure comment on red |
| **`orchestrator.sh`** | The grand loop: pick → dispatch → sub-agent → ready-or-bail |

## Four-layer guardrail architecture

End-to-end autonomy without a human in the loop means the guardrail *must*
catch regressions the agent can't see itself. We enforce at four layers:

1. **Pre-dispatch** (`check-overlaps.sh` + `pick-ticket.sh`) — refuse to
   start work that would collide with another open PR or an active claim.
2. **In-agent** — `CLAUDE.md` in the repo root tells the sub-agent the
   rules (no test edits, no new `unsafe`, no CI edits, bounded blast
   radius). Self-enforcement; cheap and fast.
3. **Pre-ready** (`verify-scope.sh` via `ready-or-bail.sh`) — the
   gatekeeper. Only this script marks a draft PR ready. If it fails, the
   PR stays draft with a structured failure comment.
4. **CI** (`.github/workflows/agent-guardrails.yml`) — re-runs
   `verify-scope.sh`'s diff-shape + overlap checks on GitHub so the
   guardrail can't be bypassed by a local run.

Layers 1–2 are best-effort. Layer 3 is the contract — a draft PR
transitions to ready **only** through `ready-or-bail.sh`. Layer 4 is the
belt-and-suspenders.

## Typical autonomous loop

```bash
# One-shot: pick one ticket, dispatch, run a sub-agent, gate the PR.
agent-scripts/orchestrator.sh --n 1

# Three tickets in parallel, worktree-isolated.
agent-scripts/orchestrator.sh --n 3 --parallel 3

# Drain the queue.
agent-scripts/orchestrator.sh --loop

# Plan without mutating anything.
agent-scripts/orchestrator.sh --dry-run
```

Under the hood each iteration does:

```bash
issue=$(agent-scripts/pick-ticket.sh | cut -f1)
agent-scripts/dispatch-agent.sh --issue "$issue"
# ... sub-agent runs in the new worktree ...
agent-scripts/ready-or-bail.sh --pr <pr-number>
```

## Guardrail rules (`verify-scope.sh`)

| Rule | Reason |
|---|---|
| No modifications to `resilient/tests/*.rs`, `resilient-runtime/tests/*.rs`, `fuzz/fuzz_targets/*` | Test protection — see `CLAUDE.md` |
| No modifications to `*.expected.txt` goldens | Same; goldens are contract |
| No new `unsafe` blocks | Security rules in `CLAUDE.md` |
| No edits under `.github/workflows/` | CI integrity |
| ≤ `AGENT_MAX_FILES` (default 60) files touched | Bounded blast radius |
| No non-patch Cargo.lock bumps | Supply-chain hygiene |
| `cargo fmt --check` clean | Code standards |
| `cargo clippy -D warnings` clean | Code standards |
| `cargo test` passing | Never weaken a test |
| No file overlap with other open PRs | Reduces rebase churn |

Skip expensive checks on CI (which already runs fmt/clippy/test on its
own pipeline) with `--skip tests --skip clippy --skip fmt`.

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
