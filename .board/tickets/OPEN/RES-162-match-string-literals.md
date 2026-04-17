---
id: RES-162
title: Match against string literal patterns
state: OPEN
priority: P3
goalpost: G13
created: 2026-04-17
owner: executor
---

## Summary
`case "hello" => ...` is natural for text-dispatch code (option
parsing, protocol codes, small state machines). Adds no new
algorithmic complexity — string equality at each arm.

## Acceptance criteria
- Parser: string literal at pattern position.
- Exhaustiveness: over the implicit infinite space of String, a
  literal-only match is never exhaustive without `_` — same rule
  as Int today.
- Unit tests covering success, fallthrough to `_`, escape handling
  in the literal pattern (`"a\n"` matches the same string).
- Commit message: `RES-162: string-literal match patterns`.

## Notes
- Don't introduce regex patterns here — that's a separate
  decision the language hasn't made yet, and it pulls in a
  runtime dependency.
- The interpreter, VM, and JIT all already handle string equality;
  match compilation just emits the same sequence of
  `if s == "pat"` checks.

## Log
- 2026-04-17 created by manager
