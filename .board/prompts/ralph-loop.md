# Ralph Loop

Reusable Codex prompt for continuous, high-leverage improvement passes on
Resilient.

## How to reuse

Ask Codex to use this prompt again by referencing the file directly:

> Use the Ralph Loop prompt at `.board/prompts/ralph-loop.md`.

You can also paste the prompt body below into a fresh Codex turn if you
want the behavior inline.

## Operating stance

You are running an open-ended improvement loop for the Resilient
repository.

Do not optimize for tiny, isolated fixes. Prefer improvements that remove
an entire class of bugs, collapse repeated manual work, unify duplicated
flows, or unlock a larger chunk of the roadmap.

If a small issue is the highest-leverage way to unblock a bigger
improvement, fix it immediately. Otherwise, aim for the largest safe
change you can land end-to-end in the current pass.

Treat language features, workflow automation, CI, documentation, ticket
flow, release mechanics, and developer ergonomics as equally valid
targets. If the repo has a mismatch between docs and scripts, fix the
source of truth first.

## Loop contract

For each pass:

1. Inspect the current state.
2. Identify the highest-leverage improvement available.
3. Implement it end-to-end.
4. Verify it with the strongest practical checks.
5. Record the result and the next improvement target.
6. Immediately continue to the next pass unless you are blocked by an
   external dependency or the user explicitly stops the loop.

## What counts as a good pass

Favor changes that:

- remove a correctness hole,
- eliminate repeated manual steps,
- unify competing sources of truth,
- improve claim / PR / issue / branch automation,
- increase the size or quality of a future ticket stream,
- or make the next iteration materially easier.

Prefer compound improvements over cosmetic tweaks. If the current pass can
solve a workflow inconsistency and harden the related CI or orchestration
path at the same time, do both.

## Guardrails

Follow the repository rules in `AGENTS.md`, `CLAUDE.md`, and
`CONTRIBUTING.md`.

- Do not weaken tests to make a pass look good.
- Do not bypass CI or guardrails.
- Do not introduce `unsafe` without a soundness rationale.
- Do not create a second canonical source of truth for tickets or
  workflow state.

## Suggested working rhythm

1. Read `agent-scripts/README.md`, `docs/AGENT_PLAYBOOK.md`,
   `docs/agent-orchestration.md`, `README.md`, `ROADMAP.md`, and the live
   issue / PR state.
2. Pick the biggest leverage gap, not the easiest cosmetic issue.
3. Ship the fix with tests, docs, or workflow updates as needed.
4. Re-run the loop and look for the next high-value improvement.

## Exit condition

This prompt is designed to be reused. Keep looping until the user says
stop, the repo state is genuinely blocked by an external dependency, or no
meaningful improvement remains in scope for the current pass.
