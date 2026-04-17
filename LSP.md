# Resilient LSP (Language Server Protocol)

Resilient ships an **opt-in** Language Server that provides red-squiggle
diagnostics in any editor that speaks LSP. It's scoped to `did_open` +
`did_change` → `publishDiagnostics` today — no hover, no completion,
no go-to-definition yet. Those are planned follow-ups.

## Build

The LSP server pulls in `tower-lsp` + `tokio` as heavy transitive
dependencies, so it's gated behind a feature flag. Default builds
don't pay the cost.

```bash
cd resilient
cargo build --features lsp --release
# Binary lands at resilient/target/release/resilient
```

Running `resilient --lsp` without the feature emits a helpful error
and exits non-zero.

## How it works

When a buffer opens or changes, the server:

1. Runs the hand-rolled parser on the full text.
2. If parsing succeeded, runs the typechecker via
   `check_program_with_source(program, uri)`. The typechecker's
   error messages come pre-formatted with `<uri>:<line>:<col>:` thanks
   to RES-080, so the LSP server parses that prefix back into a
   structured `Range` and publishes a diagnostic at the right spot.
3. Parser errors are published at line 1, column 1 for now — the
   hand-rolled parser doesn't expose its error positions as
   structured data yet. Follow-up tickets will thread those through
   just like the typechecker errors.

## Editor config

### Neovim (with `nvim-lspconfig`)

```lua
local lspconfig = require("lspconfig")
local configs = require("lspconfig.configs")

if not configs.resilient then
  configs.resilient = {
    default_config = {
      cmd = { "/absolute/path/to/resilient", "--lsp" },
      filetypes = { "resilient" },  -- or "rs" if you map .rs files to the language
      root_dir = lspconfig.util.root_pattern("Cargo.toml", ".git"),
      settings = {},
    },
  }
end
lspconfig.resilient.setup({})
```

### VS Code (via `vscode-languageclient`)

Create a minimal extension (or use a generic LSP runner) pointing
`command` at the Resilient binary and passing `--lsp` as an argument.

## What's next

- **Parser spans in diagnostics**: thread `record_error`'s position
  data through so parser errors light up the right token.
- **Hover support**: show the inferred type of the identifier under
  the cursor.
- **Go-to-definition**: jump from a call site to the function
  declaration using the contract table.
- **Integration test**: spawn the binary, send a hand-rolled
  `initialize`/`didOpen` over the LSP framing (`Content-Length:
  ...\r\n\r\n<json>`), assert diagnostics come back.
