# Resilient LSP (Language Server Protocol)

Resilient ships a Language Server that provides diagnostics, hover,
go-to-definition, and completion in any editor that speaks LSP.

**Pre-built release binaries** (see the README "Install" section)
ship with the LSP compiled in already (RES-4002) — `rz --lsp` works
immediately after `curl`-installing or downloading a release tarball,
no rebuild needed. CI verifies this on every tagged release via
`scripts/release-lsp-smoke-test.sh`, which sends a JSON-RPC
`initialize` request over stdio and checks the response for a
`capabilities` object.

## Build from source

The LSP server pulls in `tower-lsp` + `tokio` as heavy transitive
dependencies (~2.4 MB added to the release binary), so a **from-source
build** keeps it behind an opt-in feature flag rather than defaulting
it on — that keeps `cargo build`/`cargo test` in this repo, and any
downstream `cargo install --path resilient` without the flag, free of
that dependency tree.

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
- **Hover** (RES-181, RES-302) — literal tokens (`int`, `float`,
  `string`, `bool`) get an exact type read from the lexer; identifiers
  fall back to the inferred type from the cached AST.
- **Go-to-definition** (RES-182, RES-3135) — jumps to the top-level
  `fn` / `struct` / `type` alias under the cursor, including across
  files reachable via the current document's `use "..."` graph. Local
  bindings and parameters still return "no definition found."
- **Find references** (RES-183) — collects every call site whose
  callee matches the cursor's top-level function name (struct
  literals with the same name are excluded via AST matching, not text).
- **Rename** (RES-184, RES-2568b) — workspace-wide rename with
  `prepareRename` support, so clients surface "cannot rename here"
  before the user starts typing. Cross-file rename scans the on-disk
  workspace, not just open buffers.
- **Code actions** (RES-357) — quick-fix light bulb for the L0010
  "add contract stubs" diagnostic.
- **Completion** (RES-188) — builtins (alphabetical) followed by
  top-level declarations in the current file. No scope-aware local
  or parameter completion yet, and no post-dot field-completion
  trigger (see "What's next").
- **Inlay hints** (RES-3135 family) — inferred `let` types and
  inferred function return types, both individually toggleable via
  client `initializationOptions`.
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

Use the bundled `vscode-extension/` workspace or any generic LSP runner
extension. Point `command` at the Resilient binary and pass `--lsp` as
the argument.

## What's next

- Go-to-definition and find-references for local bindings and
  parameters (currently top-level declarations only — needs a
  scope-aware resolver).
- Scope-aware local-variable and parameter completion.
- Post-dot field completion for structs (`p.` → field list).
- Finer-grained parser error positions.
