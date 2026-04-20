---
id: RES-220
title: GitHub issue templates + agent-ready label for autonomous contributor pipeline
status: OPEN
labels: [infra, dx]
roadmap: G20
---

## Goal

Make it easy for autonomous AI agents (Claude Code, OpenClaw, Codex, etc.) and
human contributors to discover well-scoped work and file new tickets in a
structured format that agents can parse and act on.

## Files to touch

- `.github/ISSUE_TEMPLATE/agent-ready-ticket.yml` — structured template for new tickets
- `.github/ISSUE_TEMPLATE/bug-report.yml` — bug report template
- `.github/ISSUE_TEMPLATE/config.yml` — disable blank issues, add discussion link
- `CONTRIBUTING.md` — expand "For AI Agent Contributors" section with quick-start checklist
- `.board/tickets/OPEN/` — add this ticket file

## Acceptance criteria

- [ ] GitHub "New Issue" UI shows two template choices: "Agent-Ready Ticket" and "Bug Report"
- [ ] Agent-ready template fields: Ticket ID, Goal, Files to touch, Acceptance criteria, Out of scope, Roadmap goal
- [ ] Existing 14 open issues carry the `agent-ready` label
- [ ] CONTRIBUTING.md "For AI Agent Contributors" includes a numbered quick-start checklist
- [ ] `good-first-issue` and `agent-ready` labels exist in the repo

## Out of scope

- Auto-assigning issues to agents
- Bot auto-triage
- Changes to the .board/ ticket file format
