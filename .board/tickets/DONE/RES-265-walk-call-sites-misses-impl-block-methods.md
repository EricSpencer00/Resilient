---
id: RES-265
title: "LSP find-references: walk_call_sites doesn't descend into ImplBlock methods"
state: OPEN
priority: P3
goalpost: G17
created: 2026-04-20
owner: executor
Claimed-by: Claude
closed-commit: 04d966897792ef95f6a954f29213705669459b89
---

## Summary

`walk_call_sites` in `resilient/src/lsp_server.rs` (the AST walker powering
`textDocument/references`) does not have an explicit match arm for
`Node::ImplBlock`. Any function call that appears inside an `impl` block
method body falls through to `_ => {}` and is silently missed.

## Affected code

`resilient/src/lsp_server.rs` — `fn walk_call_sites` (line ~467).
The catch-all `_ => {}` arm at line ~623 silently drops `Node::ImplBlock`.

## Example

```resilient
fn helper() -> int { 1 }

struct Point { x: int, y: int }

impl Point {
    fn sum(self) -> int {
        return helper() + helper();   // both calls missed by find-references
    }
}
```

`textDocument/references` on `helper` returns zero results — the two calls
inside the `impl` method are invisible.

## Acceptance criteria

- `walk_call_sites` gains an explicit arm for `Node::ImplBlock { methods, .. }`:
  iterate over `methods` and recurse into each method body.
- New unit test `references_finds_call_inside_impl_method` in
  `lsp_server.rs` under `#[cfg(test)]`.
- Existing find-references tests continue to pass.
- `cargo test` passes with 0 failures.
- `cargo clippy --all-targets -- -D warnings` clean.
- Commit: `RES-265: walk_call_sites — descend into ImplBlock methods`.

## Notes

`Node::ImplBlock` is defined around line 1370 of `main.rs`:
`ImplBlock { struct_name, methods: Vec<Node>, span }`.
Each element of `methods` is a `Node::Function`, so the fix is a single
`for method in methods { walk_call_sites(method, target, out); }` loop.

## Log

- 2026-04-20 created by analyzer
