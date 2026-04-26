# Resilient LSP (Language Server Protocol)

Resilient ships an **opt-in** Language Server that provides diagnostics,
hover, go-to-definition, and completion in any editor that speaks LSP.

## Build

The LSP server pulls in `tower-lsp` + `tokio` as heavy transitive
dependencies, so it's gated behind a feature flag.

```bash
cd resilient
cargo build --features lsp --release
# Binary lands at resilient/target/release/rz
```

Running `resilient --lsp` without the feature emits a helpful error
and exits non-zero.

## Start the server

```bash
resilient --lsp
```

Communicates over stdin/stdout using standard JSON-RPC LSP framing.

## What's implemented

- **Diagnostics** — `did_open` + `did_change` → full parse + typecheck
  → `publishDiagnostics` with `<uri>:<line>:<col>:` locations.
- **Hover** (RES-181) — shows the type of any literal token under the
  cursor (`int`, `float`, `string`, `bool`).
- **Go-to-definition** (RES-182) — jumps to the top-level declaration
  of the function or struct name under the cursor.
- **Completion** (RES-188) — builtins (alphabetical) followed by
  top-level declarations in the current file.
- **Semantic tokens** (RES-187) — structured token types for editors
  that support semantic highlighting.

## Editor config

### Neovim (with `nvim-lspconfig`)

```lua
local lspconfig = require("lspconfig")
local configs = require("lspconfig.configs")

if not configs.resilient then
  configs.resilient = {
    default_config = {
      cmd = { "/absolute/path/to/resilient", "--lsp" },
      filetypes = { "rust" },  -- .rs files; adjust if you register a custom filetype
      root_dir = lspconfig.util.root_pattern("Cargo.toml", ".git"),
      settings = {},
    },
  }
end
lspconfig.resilient.setup({})
```

### VS Code

Use the bundled `vscode-extension/` scaffold or any generic LSP runner
extension. Point `command` at the Resilient binary and pass `--lsp` as
the argument.

## What's next

- Hover for identifiers (variables, parameters, function names).
- Scope-aware local-variable completion.
- Post-dot field completion for structs.
- Finer-grained parser error positions.
- Multi-file workspace go-to-definition.
