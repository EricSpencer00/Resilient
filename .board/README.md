# Resilient Development Board

A self-maintained task tracker for the Resilient programming language.
Think of it as a tiny filesystem-backed JIRA, driven by two Claude Code agents
running in `ralph`-style loops.

## Layout

```
.board/
├── README.md           — this file
├── ROADMAP.md          — goalposts (north star). Updated by Manager.
├── tickets/
│   ├── OPEN/           — tickets waiting to be picked up (highest priority first)
│   ├── IN_PROGRESS/    — ticket currently being worked
│   └── DONE/           — completed tickets (source of truth for what's shipped)
├── prompts/
│   ├── manager.md      — prompt driving the Manager loop
│   └── executor.md     — prompt driving the Executor loop
├── scripts/
│   ├── run-manager.sh  — launches a manager ralph loop
│   ├── run-executor.sh — launches an executor ralph loop
│   └── new-ticket.sh   — helper to mint a ticket with a fresh RES-N id
└── logs/               — per-iteration output from each loop
```

## Ticket format

Each ticket is a markdown file under `tickets/<STATE>/` named
`RES-<n>-<kebab-slug>.md`. It has frontmatter plus body:

```markdown
---
id: RES-007
title: Short human-readable title
state: OPEN              # OPEN | IN_PROGRESS | DONE | BLOCKED
priority: P0             # P0 (drop-other-stuff) | P1 (next) | P2 | P3
goalpost: G3             # which ladder goalpost this maps to
created: 2026-04-16
owner: executor          # or "manager" for meta/plan tickets
---

## Summary
What and why, in one paragraph.

## Acceptance criteria
- Bullet list of verifiable conditions
- `cargo test` passes, etc.

## Notes
Anything useful — prior attempts, related files, pitfalls.

## Log
- 2026-04-16 created by manager (seed)
```

State moves by **moving the file** between `tickets/OPEN|IN_PROGRESS|DONE`
directories. `mv` is atomic on one filesystem and prevents both agents from
picking up the same ticket.

## Commit discipline

- Every code change references the ticket id: `RES-007: <imperative summary>`
- Every ticket movement is committed in the same logical unit as the code it
  unblocks, so `git log --oneline --grep RES-007` tells the whole story.

## Agent contract (at a glance)

| | Reads | Writes |
|---|---|---|
| **Manager** | source, git log, tickets/DONE, ROADMAP | ROADMAP, tickets/OPEN (mints new), tickets/DONE → OPEN on rejection |
| **Executor** | tickets/OPEN, source | source code, tickets/IN_PROGRESS ↔ DONE, git commits |

Both agents run `cargo build` and `cargo test` to verify work. Neither touches
the other's write surface. If they ever conflict on git, the loser retries.
