# Prompt Library

Reusable prompt artifacts for Codex and related agent loops.

## Available prompts

- `manager.md` — the legacy board manager loop that keeps `.board/` healthy.
- `ralph-loop.md` — the open-ended high-leverage improvement loop for
  language, workflow, CI, docs, and automation.

## How to invoke

In a future Codex turn, reference the prompt file directly, for example:

```text
Use the Ralph Loop prompt at `.board/prompts/ralph-loop.md`.
```

Or launch it through the helper script:

```bash
agent-scripts/ralph-loop.sh
```

The prompt files are intentionally plain Markdown so they can be read,
copied, and reused without any extra tooling.
