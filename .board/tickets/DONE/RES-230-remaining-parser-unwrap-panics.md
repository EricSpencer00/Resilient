---
id: RES-230
title: Four remaining `unwrap()` panics in production parser paths
state: DONE
claimed-by: Claude Sonnet 4.6
priority: P2
goalpost: G3
created: 2026-04-20
owner: executor
---

## Summary
After the known parser panics were eliminated by RES-016 and RES-009,
four `.unwrap()` calls on `Option<Node>` remain in production parsing
code (i.e., outside `#[cfg(test)]`). Each can panic if
`parse_expression` returns `None` (which it does when it encounters
unexpected tokens).

## Affected locations in `src/main.rs`

| Line  | Function                  | Context                                 |
|-------|---------------------------|-----------------------------------------|
| 2529  | `parse_let_statement`     | `let value = self.parse_expression(0).unwrap();` |
| 3568  | `parse_infix_expression`  | `let right = self.parse_expression(precedence).unwrap();` |
| 3600  | `parse_call_arguments`    | `args.push(self.parse_expression(0).unwrap());` |
| 3605  | `parse_call_arguments`    | `args.push(self.parse_expression(0).unwrap());` (inside `while` loop) |

## Impact
Crafted or malformed input can crash the compiler with an unwrap panic
rather than emitting a clean diagnostic. Example inputs that trigger
each:

- **`parse_let_statement` (line 2529)**: `let x = ;` — semicolon is
  not a valid expression start, so `parse_expression` returns `None`,
  `unwrap()` panics.
- **`parse_infix_expression` (line 3568)**: `1 + ;` — same pattern.
- **`parse_call_arguments` (lines 3600/3605)**: `f(,)` or `f(;)`.

## Acceptance criteria
Replace each `.unwrap()` with either:
- `.unwrap_or(Node::IntegerLiteral { value: 0, span: ... })` (matching
  the recovery pattern used elsewhere in the parser, e.g. lines
  1576, 1620, 2164, 2170, 2329, 3266), **or**
- A `?`-return from the surrounding `Option<Node>`-returning function
  (preferred for `parse_infix_expression` and
  `parse_call_arguments` which already return `Option<Node>`).

For `parse_let_statement` (returns `Node`, not `Option<Node>`), the
recovery default is appropriate.

- All four sites use recovery rather than panic.
- Add unit tests that verify each input (`let x = ;`, `1 + ;`,
  `f(,)`) produces a parse error diagnostic rather than a panic.
- `cargo test` must remain fully green.
- `cargo clippy --all-targets -- -D warnings` must be clean.
- Commit message: `RES-230: replace remaining parser unwrap panics with recovery`.

## Notes
- Do **not** modify existing tests — add only new ones.
- The fuzz harness (RES-201) already exercises these paths; this
  ticket provides deterministic unit-test coverage and the fix.
- Only these four sites need to change — the other `.unwrap()` calls
  in the file are inside `#[cfg(test)]` (acceptable per CLAUDE.md)
  or use `.unwrap_or` already.

## Log
- 2026-04-20 created by analyzer
- 2026-04-20 fixes and tests implemented; cargo fmt applied across all files; closed
