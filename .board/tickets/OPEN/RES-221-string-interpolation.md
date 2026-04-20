---
id: RES-221
title: String interpolation — `"Hello, {name}!"` syntax
state: OPEN
priority: P3
goalpost: G11
created: 2026-04-20
owner: executor
---

## Summary
Add string interpolation so users can embed expressions directly in string literals without manual concatenation. The syntax `"Hello, {expr}!"` evaluates each `{...}` expression and splices its string representation into the result.

## Acceptance criteria
- Parser recognizes `{expr}` inside a double-quoted string and produces an `Expr::Interpolated` node containing a sequence of literal string segments and sub-expressions.
- The interpreter evaluates each sub-expression and calls the same `to_string` coercion used by `println!`, splicing the results together.
- Nested braces are an error: emit a parse error with a clear diagnostic. Escape `{` as `\{` if needed.
- Works in all positions where a string literal is valid: `let`, function arguments, `println!`, `assert`, `return`.
- Golden test: `interpolation.rs` / `interpolation.expected.txt` covering simple variable, arithmetic sub-expression, and nested function call.
- Unit test in `lexer_logos.rs` and `compiler.rs` covering the new node type.
- VM and JIT backends handle `Expr::Interpolated`.
- Commit message: `RES-221: string interpolation {expr} inside double-quoted strings`.

## Notes
- Keep implementation inside the existing recursive-descent parser — no need for a separate lexer mode; scan for `{` / `}` during string literal parsing.
- If an interpolated sub-expression fails to evaluate, propagate the error normally.

## Log
- 2026-04-20 created by manager
</content>
</invoke>