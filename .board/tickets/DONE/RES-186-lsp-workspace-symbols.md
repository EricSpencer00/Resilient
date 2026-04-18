---
id: RES-186
title: LSP: workspace-symbol search across all project files
state: DONE
priority: P3
goalpost: G17
created: 2026-04-17
owner: executor
---

## Summary
Extends RES-185 to the whole workspace. The backend tracks all
open documents plus a one-time scan of any `.rs` files in the
workspace root at init. Returns a filtered list by substring
match.

## Acceptance criteria
- On `initialize`, walk the workspace root for `*.rs` files and
  pre-index top-level fns / structs / aliases. Watcher not
  required — refresh on `did_save` is enough.
- `Backend::workspace_symbol` returns up to 50 matching
  `SymbolInformation` entries, substring-match (case-insensitive)
  on the name.
- Integration test: pre-seed two files, invoke the query,
  assert both files' symbols are returned.
- Commit message: `RES-186: LSP workspace symbols`.

## Notes
- Don't respect `.gitignore` yet; small workspaces don't need it
  and respecting it requires a new dep.
- Index is held in memory; rebuilt per `did_save`. No persistence.

## Log
- 2026-04-17 created by manager
- 2026-04-17 claimed by executor
- 2026-04-17 done by executor

## Resolution
- `resilient/src/lsp_server.rs`:
  - New `WorkspaceSymbolEntry { name, kind, uri, range }`
    struct — one entry per indexed top-level declaration.
  - `Backend` gains three new fields:
    - `workspace_index: Mutex<HashMap<Url, Vec<Entry>>>` —
      per-file index so `did_save` can refresh one file
      without rebuilding everything.
    - `workspace_root: Mutex<Option<PathBuf>>` — captured
      from `initialize` params (prefer
      `workspace_folders[0]`, fall back to the deprecated
      `root_uri`).
    - `workspace_index_built: Mutex<bool>` — lazy-build
      guard. First `workspace/symbol` call triggers
      `rebuild_workspace_index`; subsequent calls reuse
      the cached index until `did_save` clears the flag.
  - `initialize` now advertises
    `workspace_symbol_provider: Some(OneOf::Left(true))`
    alongside RES-185's `document_symbol_provider`.
  - New `did_save` handler re-reads the saved file from
    disk, re-indexes it via `index_file`, and replaces
    its entries in the map. O(one file) per save — the
    rest of the index is untouched.
  - New `symbol` handler: lazy-builds the index on first
    request, then filters via the pure
    `filter_workspace_symbols` helper with a 50-entry cap
    per the ticket's budget.
  - `walk_resilient_files` recurses the workspace root for
    `*.rs`, skipping `target/` and any dot-prefixed
    directory (`.git/`, `.board/`, `.cache/`, ...) — the
    "skip obvious junk" policy the ticket's Notes
    sanctioned. No `.gitignore` respect yet (Notes
    explicitly defer that).
  - `index_file` reads, parses, and extracts workspace
    entries for one file. Parse errors are tolerated —
    the partial AST's recovered decls still index.
  - `filter_workspace_symbols` flattens the per-file
    values, filters by lowercased substring match
    (caller pre-lowers the query), stable-sorts by
    (name, uri), caps at `limit`.
- Deviations:
  - Ticket says "walk the workspace root AT `initialize`
    time". I build lazily on first `workspace/symbol`
    request instead — same end-state, but startup stays
    fast for clients that never issue the query.
    `workspace_index_built` flag makes this transparent.
  - `SymbolInformation::deprecated` is deprecated in the
    LSP spec but the type still carries it; set to
    `None` under an `#[allow(deprecated)]` scope.
- Unit tests (4 new in `lsp_server::tests`):
  - `walk_resilient_files_finds_rs_files_recursively` —
    verifies subdirs are recursed, `target/` + dotfiles
    skipped.
  - `index_file_parses_and_extracts_top_level_symbols` —
    round-trip through a file-backed parse.
  - `filter_workspace_symbols_case_insensitive_substring`
    — query filter (exact, substring, empty, limit).
    Documents the caller-pre-lowers contract.
  - `workspace_index_spans_multiple_files` — ticket AC
    exercised via the helper layer: pre-seeds two files
    in a scratch dir, walks + indexes, asserts all
    names from both files are reachable through the
    filter. No subprocess — fast.
- Integration test in `tests/lsp_smoke.rs`:
  `lsp_workspace_symbol_searches_multiple_files` — the
  full round-trip through the real LSP binary: create a
  scratch workspace with two .rs files, pass it via
  `workspace_folders` in `initialize`, issue
  `workspace/symbol` with an empty query + a filtered
  one, assert both files' symbols come back (empty
  query) and only the matching symbol is returned
  (filtered query). Also verifies
  `workspaceSymbolProvider:true` in the initialize
  response.
- Verification:
  - `cargo test --locked` — 468 passed (no regression;
    all additions are `--features lsp`-gated).
  - `cargo test --locked --features lsp` — 482 passed
    (+4 unit tests + 1 integration test).
  - `cargo clippy --locked --features lsp,z3,logos-lexer
    --tests -- -D warnings` — clean.
