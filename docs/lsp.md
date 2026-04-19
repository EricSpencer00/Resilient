---
title: Editor Integration (LSP)
nav_order: 7
permalink: /lsp
---

# Editor Integration (LSP)
{: .no_toc }

Resilient ships an opt-in Language Server that wires up red-squiggle
diagnostics, hover, go-to-definition, and completion in any editor
that speaks LSP.
{: .fs-6 .fw-300 }

<details open markdown="block">
  <summary>Table of contents</summary>
  {: .text-delta }
- TOC
{:toc}
</details>

---

## Build

The LSP server pulls in `tower-lsp` + `tokio`, so it's gated behind
a feature flag. Default builds don't pay the cost.

```bash
cd resilient
cargo build --features lsp --release
# Binary: resilient/target/release/resilient
```

Running `resilient --lsp` without the feature emits a helpful error
and exits non-zero.

## Start the server

```bash
resilient --lsp
```

The server communicates over stdin/stdout using the standard
JSON-RPC LSP framing. Your editor client manages the process.

---

## What's implemented

### Diagnostics

Every `did_open` or `did_change` event re-runs the full parse +
typecheck pipeline and publishes structured diagnostics with
`<uri>:<line>:<col>:` locations. Editor squiggles appear in the
correct column for type errors. Parser errors currently appear at
line 1, column 1 — finer-grained parser spans land in a follow-up.

### Hover

Hovering over any **literal token** shows its type:

| Literal | Hover shows |
|---------|-------------|
| `42`    | `int`       |
| `3.14`  | `float`     |
| `"hi"`  | `string`    |
| `true`  | `bool`      |

Hover over identifiers (variables, functions) is a planned
follow-up that depends on the full type-inference pass.

### Go-to-definition

Clicking "go to definition" on any **top-level function or struct
name** jumps to its declaration site. Works across a single file;
multi-file workspace lookup is a follow-up.

### Completion

Triggering completion offers:

1. All built-in functions, alphabetical (`abs`, `ceil`, `floor`, …).
2. Every top-level declaration in the current file (functions,
   structs, type aliases), alphabetical after builtins.

Scope-aware local-variable completion and post-dot field completion
are follow-up tickets.

---

## Editor setup

### Neovim (nvim-lspconfig)

```lua
local lspconfig = require("lspconfig")
local configs   = require("lspconfig.configs")

if not configs.resilient then
  configs.resilient = {
    default_config = {
      cmd      = { "/absolute/path/to/resilient", "--lsp" },
      filetypes = { "rust" },   -- .rs files; change to "resilient" if you register a custom filetype
      root_dir  = lspconfig.util.root_pattern("Cargo.toml", ".git"),
      settings  = {},
    },
  }
end
lspconfig.resilient.setup({})
```

Replace `/absolute/path/to/resilient` with the path to your built
binary (e.g. `~/GitHub/Resilient/resilient/target/release/resilient`).

### VS Code

Use the bundled `vscode-extension/` or any generic LSP runner
extension (e.g. *Generic LSP Client*). Point `command` at the
Resilient binary with `--lsp` as the argument and set the language
ID to `rust` (or register a custom `resilient` language ID).

The `vscode-extension/` directory in the repo contains a minimal
extension scaffold — `npm install && vsce package` inside it
produces an installable `.vsix`.

---

## Semantic tokens

The server also serves `textDocument/semanticTokens/full` — editors
that support semantic highlighting will color keywords, literals,
type names, and function calls distinctly beyond what syntax
highlighting alone provides.

---

## What's next

- Hover for identifiers (variables, parameters, function names).
- Scope-aware local-variable completion.
- Post-dot field completion for structs.
- Finer-grained parser error positions.
- Multi-file workspace go-to-definition.
