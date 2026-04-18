---
id: RES-204
title: VS Code extension package connecting to the LSP server
state: IN_PROGRESS
priority: P3
goalpost: tooling
created: 2026-04-17
owner: executor
---

## Summary
We have an LSP server (RES-074 + the feature work under RES-181..190).
Users need a client. Ship a minimal VS Code extension that
launches the binary with `--lsp` and activates on `.rs` files
inside Resilient projects.

## Acceptance criteria
- New directory `vscode-extension/` at repo root:
  - `package.json` with `activationEvents: ["onLanguage:resilient"]`
    (new language id), `main: ./out/extension.js`,
    `engines.vscode: ^1.80.0`.
  - `src/extension.ts` starts a LanguageClient pointing at
    `resilient --lsp`, with a server options path resolvable via
    a workspace setting `resilient.serverPath` (defaults to
    `resilient` on PATH).
  - `syntaxes/resilient.tmLanguage.json` minimal TextMate grammar
    for keyword / string / number coloring (semantic tokens from
    RES-187 refines on top).
- README in the extension dir explains development + publishing
  (via `vsce package`). Publishing itself is out of scope — we
  generate a .vsix artifact in CI.
- CI job that builds the extension + uploads the .vsix on tag push.
- Commit message: `RES-204: VS Code extension package`.

## Notes
- "resilient" as a language ID collides with nothing on the VS
  Code marketplace today; claim it now.
- Don't bundle the compiler binary into the extension — too
  platform-y. The setting indirection lets power users point at
  dev builds.

## Log
- 2026-04-17 created by manager
- 2026-04-17 claimed by executor
