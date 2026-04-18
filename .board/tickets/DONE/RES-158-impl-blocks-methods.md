---
id: RES-158
title: `impl Point { fn mag(self) -> Float { ... } }` methods on structs
state: DONE
priority: P2
goalpost: G12
created: 2026-04-17
owner: executor
---

## Summary
Today `distance(p1, p2)` is the only way to organize struct-related
code. Method syntax `p1.distance(p2)` is immediately more
readable, and matches what users of every modern language expect.

## Acceptance criteria
- Parser: `impl <StructName> { <fn_decl>* }` at top level. Each
  `fn_decl` accepts `self` as the first parameter, typed as the
  enclosing struct.
- Method call: `p.mag()` desugars to `Point$mag(p)` post-resolution.
  Dispatch is static (no vtables — we're nominal).
- `self` is immutable by default. Mutation inside a method mutates
  a local copy unless the call site assigns the result back, same
  as current parameter semantics. Document this clearly.
- Unit tests: method definition, method call, method calling
  another method on the same struct.
- Commit message: `RES-158: impl blocks for struct methods`.

## Notes
- This is sugar, not a dispatch system — it produces exactly the
  same bytecode / JIT output as a free function. No perf
  implications.
- Multiple `impl Point { ... }` blocks allowed, collected at
  resolution time. Same-name method across blocks is a duplicate-def
  diagnostic.

## Log
- 2026-04-17 created by manager
- 2026-04-17 claimed by executor
- 2026-04-17 done by executor

## Resolution

Files changed:
- `resilient/src/main.rs`
  - Added `Token::Impl` + lexer `"impl"` keyword arm.
  - Added `Node::ImplBlock { struct_name, methods, span }`.
  - Added `parse_impl_block` + `parse_method` — the parser emits
    each method as a `Node::Function` with a mangled name
    (`<StructName>$<method>`) and `self` (if present as the first
    parameter) injected as `(<StructName>, "self")`. Static /
    associated functions without `self` fall through to the normal
    parameter-list parser.
  - `parse_statement` dispatches on `Token::Impl` to the new parser.
  - Interpreter: `eval_program` hoists `ImplBlock` methods alongside
    top-level `fn` decls so call sites can forward-reference them;
    `ImplBlock` eval registers each method in the env and emits a
    duplicate-method diagnostic if the same mangled name already
    resolves to a user function.
  - Interpreter: `CallExpression` with a `FieldAccess` callee now
    desugars — when the target evaluates to `Value::Struct { name, .. }`,
    looks up `<name>$<method>` in the env and calls it with the
    target as the implicit `self`, followed by the call-site args.
    Unknown methods fall through to the regular `FieldAccess` error
    path.
  - Three new unit tests in `mod tests`:
    `impl_block_method_call_dispatches_to_mangled_fn`,
    `impl_block_method_can_call_another_method_on_same_struct`,
    `impl_block_duplicate_method_is_error`.
- `resilient/src/typechecker.rs`: `Node::ImplBlock` arm walks each
  method via the regular `Node::Function` check.
- `resilient/src/compiler.rs`: added `Node::ImplBlock` to the
  `node_line` structural-variants pattern so the AST→line match
  stays exhaustive.

Acceptance criteria:
- Parser for `impl <StructName> { <fn_decl>* }` with `self` first
  param — yes.
- Method call desugars to `<Struct>$<method>(target, ...)` — yes,
  at eval time in `CallExpression`.
- `self` immutable-by-default: the method receives `self` as a
  by-value parameter like any other fn param, so mutations are
  confined to the local frame unless the caller assigns the return
  value back. Documented inline on the `self` parameter-injection
  site in `parse_method`.
- Multiple `impl Point { ... }` blocks supported; duplicate method
  across blocks is diagnosed at eval time (`duplicate method:
  Point::m defined more than once across impl blocks`).
- Unit tests: method definition + call, method-calls-another-
  method, duplicate-def diagnostic — yes, three tests.

Verification:
- `cargo build` — clean.
- `cargo test` — 271 unit (+3 new) + 13 integration pass.
- `cargo clippy --tests -- -D warnings` — clean.
- Manual: `fn mag_sq(self) -> int { return self.x * self.x + self.y * self.y; }`
  on a `Point { x: 3, y: 4 }` prints `25`; method calling another
  method on same struct (`doubled_sum` → `sum() + sum()`) prints
  the expected sum.
