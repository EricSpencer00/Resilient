# Resilient for VS Code

Minimal VS Code extension that launches the Resilient compiler as a
Language Server over stdio and binds it to `.rs` files with the
`resilient` language id.

Ships:

- Syntax highlighting via a TextMate grammar
  (`syntaxes/resilient.tmLanguage.json`). Semantic tokens from
  RES-187 refine the coloring once the LSP is connected.
- Language configuration (brackets, comments, folding) in
  `language-configuration.json`.
- A thin `LanguageClient` that spawns `resilient --lsp` and
  forwards diagnostics, document symbols, workspace symbols,
  and semantic tokens.

## Settings

| Setting                     | Default       | Purpose                                                                      |
| --------------------------- | ------------- | ---------------------------------------------------------------------------- |
| `resilient.serverPath`      | `resilient`   | Path to the `resilient` binary. Point at a dev build when hacking.           |
| `resilient.serverArgs`      | `["--lsp"]`   | Arguments passed to the binary. The server expects `--lsp`.                  |
| `resilient.trace.server`    | `off`         | `off` / `messages` / `verbose` — traces LSP traffic to an output channel.    |

## Development

```bash
# install deps
cd vscode-extension
npm install

# type-check + emit to out/
npm run compile

# open the extension dev host (VS Code, F5 from the vscode-extension
# folder, then open a .rs file in the launched window)
```

If the LSP fails to start, check the **Resilient LSP** output
channel (the extension creates it on activation). The most common
cause is `resilient.serverPath` pointing at a binary that wasn't
built with `--features lsp`; the CLI prints a helpful message if
so.

## Publishing

Publishing to the VS Code marketplace is out of scope for now. To
produce a `.vsix` artifact locally:

```bash
# one-time: install vsce via the project's devDependencies
npm install
# then
npx vsce package --out resilient-vscode.vsix
```

CI (`.github/workflows/vscode_extension.yml`) runs
`npm run compile` on every push + PR, and builds + uploads the
`.vsix` as a release artifact on tag pushes.

## File tree

```
vscode-extension/
├── README.md                            # you are here
├── package.json                         # extension manifest
├── tsconfig.json                        # TypeScript config (strict)
├── language-configuration.json          # brackets / comments
├── syntaxes/
│   └── resilient.tmLanguage.json        # TextMate grammar
├── src/
│   └── extension.ts                     # activate/deactivate + client
├── .vscodeignore                        # files excluded from .vsix
└── .gitignore                           # node_modules/, out/, *.vsix
```
