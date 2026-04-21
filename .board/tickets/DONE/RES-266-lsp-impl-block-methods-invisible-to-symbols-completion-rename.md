---
id: RES-266
title: "LSP: ImplBlock methods invisible to document symbols, completion, and rename"
state: OPEN
priority: P3
goalpost: G17
created: 2026-04-20
owner: executor
Claimed-by: Claude
closed-commit: 04d966897792ef95f6a954f29213705669459b89
---

## Summary

Three LSP helpers that enumerate top-level declarations all skip
`Node::ImplBlock`:

| Function | Effect of missing ImplBlock |
|---|---|
| `build_top_level_defs` | impl methods excluded from rename eligibility and definition lookup |
| `document_symbols_for_program` | impl methods absent from the document outline |
| `completion_candidates` | impl method names never offered as completions |

Methods defined via `impl StructName { fn method(...) }` are therefore
invisible to the LSP's outline, auto-complete, and rename workflows.

## Affected code

`resilient/src/lsp_server.rs`:
- `fn build_top_level_defs` (line ~360) — `_ => continue` arm drops `ImplBlock`
- `fn document_symbols_for_program` (line ~793) — `_ => continue` arm drops `ImplBlock`
- `fn completion_candidates` (line ~703) — `_ => continue` arm drops `ImplBlock`

## Acceptance criteria

- `document_symbols_for_program` emits a `METHOD` symbol for each method
  inside every `ImplBlock` found in the program.
- `completion_candidates` emits a `Candidate` of kind `Function` for each
  method inside every `ImplBlock`, with detail `"fn (N params) on StructName"`.
- `build_top_level_defs` includes impl methods so they appear as
  rename/definition targets. (Or alternatively, add a separate `find_method_def`
  helper and update callers; see Notes.)
- New unit tests in `lsp_server.rs` `#[cfg(test)]`:
  - `document_symbols_includes_impl_method`
  - `completion_includes_impl_method`
- Existing tests continue to pass.
- `cargo test` passes with 0 failures.
- `cargo clippy --all-targets -- -D warnings` clean.
- Commit: `RES-266: LSP — expose ImplBlock methods in symbols, completion, defs`.

## Notes

`Node::ImplBlock` carries `struct_name: String` and `methods: Vec<Node>`.
Each `methods[i]` is a `Node::Function { name, parameters, .. }`.

The mangled method name that the evaluator registers is
`struct_name__method_name` (see `main.rs` `parse_impl_block`). Depending on
whether completion should show `StructName::method` or just `method`, the
label format will need a design decision — document that choice in the PR.

## Log

- 2026-04-20 created by analyzer
