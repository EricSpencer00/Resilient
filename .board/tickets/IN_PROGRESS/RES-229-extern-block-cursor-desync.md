---
id: RES-229
title: "`parse_extern_block` advances past `}` causing cursor desync"
state: IN_PROGRESS
priority: P1
goalpost: G3
created: 2026-04-20
owner: executor
claimed-by: Claude Sonnet 4.6
---

## Summary
`parse_extern_block` calls `self.next_token()` after consuming the
closing `}` of the extern block (line 2768 of `src/main.rs`). This
violates the invariant that statement parsers leave `current_token`
**on** the closing token — `parse_program` and `parse_block_statement`
both call `self.next_token()` after each statement, so the first token
of the next declaration is consumed and skipped.

The result: any program that has statements after an `extern { ... }`
block fails to parse. The parser's cursor lands on the identifier
following `fn`, causing it to try to parse the function body as a map
literal.

## Reproduction

```
extern "libm.so.6" {
    fn sqrt(x: Float) -> Float;
}
fn main() {
    println(sqrt(16.0));
}
main();
```

```
Parser error: Expected '->' between map key and value, found `;`
Parser error: Expected '}' to close map literal, found `;`
Error: Failed to parse program: 2 parser error(s)
```

The bug can be verified with the existing `examples/ffi_libm.res`
example — `resilient examples/ffi_libm.res` exits with a parse error.

## Root cause
`parse_extern_block` (around line 2767–2768 in `src/main.rs`):

```rust
if matches!(self.current_token, Token::RightBrace) {
    self.next_token();  // <-- BUG: advances past `}`
}
```

All other top-level statement parsers (`parse_struct_decl`,
`parse_impl_block`, `parse_function_with_pure`) leave `current_token`
**on** the `}`. The outer loop in `parse_program` (and similarly
`parse_block_statement`) advances past it.

## Acceptance criteria
- Remove the `self.next_token()` from `parse_extern_block` so that
  it leaves `current_token` on the closing `}`, matching the
  convention of all other statement parsers.
- Add a unit test (in the existing `#[cfg(test)]` module) that
  parses `extern "lib" { fn f() -> Int; } fn main() { }` and asserts
  both the `Extern` node **and** the `Function` node appear in the
  program's statement list.
- `resilient examples/ffi_libm.res` must no longer produce a parse
  error (it will fail later at runtime because `libm.so.6` is only
  available on Linux, but the parse phase must succeed on all
  platforms).
- `cargo test` must remain fully green.
- Commit message: `RES-229: fix extern block cursor desync after closing \`}\``.

## Notes
- The existing tests for `parse_extern_block` only test the block
  in isolation (as the sole statement in a program). They pass
  because the double-advance only manifests when another statement
  follows.
- Do **not** modify existing tests — add only new ones.
- This is a one-line fix plus one new test.

## Log
- 2026-04-20 created by analyzer
