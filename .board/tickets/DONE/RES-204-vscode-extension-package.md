---
id: RES-204
title: VS Code extension package connecting to the LSP server
state: DONE
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

## Resolution

### Files added
- `vscode-extension/package.json` — manifest with
  `activationEvents: ["onLanguage:resilient"]`,
  `engines.vscode: ^1.80.0`, `main: ./out/extension.js`, and
  three user-facing settings: `resilient.serverPath` (defaults
  to `resilient`), `resilient.serverArgs` (defaults to
  `["--lsp"]`), `resilient.trace.server` (tri-state).
- `vscode-extension/tsconfig.json` — strict TypeScript config,
  ES2022 target, CommonJS output to `out/`.
- `vscode-extension/src/extension.ts` — `activate()` spins up a
  `LanguageClient` pointing at `resilient --lsp` over stdio,
  wires up document-sync + file watchers + trace channel;
  `deactivate()` stops the client. A spawn failure surfaces as
  an error toast pointing the user at `resilient.serverPath`.
- `vscode-extension/language-configuration.json` — brackets,
  auto-closing pairs, line + block comment markers, fold-region
  markers.
- `vscode-extension/syntaxes/resilient.tmLanguage.json` — minimal
  TextMate grammar for comments, strings (incl. `b"…"`), numeric
  literals, keywords (declaration / control / contract /
  boolean), primitive types, operators, and a
  `function-definition` pattern that tags the name after `fn` as
  `entity.name.function`. Semantic tokens from RES-187 refine on
  top.
- `vscode-extension/README.md` — dev + publishing instructions
  (`npm install && npm run compile`, `npx vsce package`), a
  settings table, a troubleshooting note pointing at the
  "Resilient LSP" output channel, and a file-tree overview.
- `vscode-extension/.vscodeignore` — excludes `src/`,
  `tsconfig.json`, `.ts`, `.map`, `node_modules/` from the
  packaged `.vsix`, keeping only `out/` JS + the grammar +
  manifest.
- `vscode-extension/.gitignore` — `node_modules/`, `out/`,
  `*.vsix`, `package-lock.json` so build artifacts don't leak
  into the repo.
- `.github/workflows/vscode_extension.yml` — two jobs: `build`
  (type-check + compile on every push + PR) and `package`
  (compile + `vsce package` + upload `.vsix` artifact — gated
  to tag pushes `v*`).

### Verification
Ran locally in `vscode-extension/`:
- `npm install --no-audit --no-fund` → 188 packages, no errors
- `npm run compile` (= `tsc -p ./`) → clean, emits
  `out/extension.js` + sourcemap
- `npx --yes @vscode/vsce package` → produced
  `resilient-vscode.vsix` (7 files, 6.24 KB) with the expected
  tree. Artifacts then removed (`.gitignore` would have kept
  them out regardless).

Cargo side:
- `cargo build` → clean
- `cargo test --locked` → 478 + 16 + 4 + 3 + 1 + 12 tests pass
  (unchanged from before — the extension is a pure-add and
  doesn't touch Rust sources)

### Notes
- Settings indirection (`resilient.serverPath`) lets power
  users point at dev builds without repackaging. Per ticket
  Notes: we deliberately do NOT bundle the compiler binary.
- `package-lock.json` is gitignored — standard for editor-
  extension projects that publish on `vsce package` (which
  falls back to `npm shrinkwrap`-style semantics anyway). CI
  does `npm install` fresh on every run.
- Shipped the `@vscode/vsce` dep-dev so `npx vsce package`
  resolves locally without asking the user to `npm i -g`
  anything.

## Log
- 2026-04-17 created by manager
- 2026-04-17 claimed by executor
- 2026-04-17 resolved by executor (extension scaffolded; tsc +
  vsce package both succeed; CI workflow added)
