---
title: Editor Integration (LSP)
parent: Language Reference
nav_order: 3
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
cargo install --path resilient --features lsp
# `rz` lands in ~/.cargo/bin/rz with LSP support compiled in.
```

Running `rz --lsp` against a build without the feature emits a helpful
error and exits non-zero.

## Start the server

```bash
rz --lsp
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

Hovering over any **literal token** shows its surface type:

| Literal | Hover shows |
|---------|-------------|
| `42`    | `Int`       |
| `3.14`  | `Float`     |
| `"hi"`  | `String`    |
| `true`  | `Bool`      |

Hovering over identifiers also returns current best-effort type or
signature information for top-level `let`, `const`, `static let`,
top-level function names, function parameters, and local `let`
bindings. Names outside those supported scopes return no hover instead
of a guessed type.

### Go-to-definition

Clicking "go to definition" on any **top-level function or struct
name** jumps to its declaration site in the current file or an imported
workspace file. Workspace lookup follows `use "..."` imports for
top-level functions and structs, including unopened files under the
initialized workspace folder.

### Find references

Running "find references" on a supported identifier returns LSP
locations for:

1. Top-level functions across the current file and imported workspace
   files.
2. Struct types across the current file and imported workspace files.
3. Same-file variable declarations, reads, and writes.

The client controls whether the declaration site is included through
the standard `includeDeclaration` request flag.

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
      cmd      = { "/absolute/path/to/rz", "--lsp" },
      filetypes = { "resilient" }, -- .rz files
      root_dir  = lspconfig.util.root_pattern("Cargo.toml", ".git"),
      settings  = {},
    },
  }
end
lspconfig.resilient.setup({})
```

Replace `/absolute/path/to/rz` with the path to your built
binary (e.g. `~/GitHub/Resilient/resilient/target/release/rz`).
If your editor does not already know the `resilient` filetype, map
`.rz` files to it before starting the client.

### VS Code

Use the bundled `vscode-extension/` or any generic LSP runner
extension (e.g. *Generic LSP Client*). Point `command` at the
`rz` binary with `--lsp` as the argument and set the language
ID to `resilient` for `.rz` files.

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

## Inlay hints

The server advertises `textDocument/inlayHint` support. Type hints are
enabled by default for inferred `let` bindings and omitted function
return types, including anonymous function literals. Disable those
hints with the initialization option
`resilient.inlayHints.types: false`.

Parameter hints for user-function call sites are opt-in. Enable them
with `resilient.inlayHints.parameters: true`.

---

## What's next

- Scope-aware local-variable completion.
- Post-dot field completion for structs.
- Finer-grained parser error positions.
