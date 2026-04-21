---
id: RES-257
title: "REPL: multi-line input via rustyline `Validator` — continuation prompt for incomplete expressions"
state: DONE
priority: P3
goalpost: tooling
created: 2026-04-20
owner: executor
Claimed-by: Claude Sonnet 4.6
Closed-by: 4015416
---

## Summary

Typing a multi-line function or block in the REPL today requires entering
everything on a single line or using semicolons as line breaks. If a user
types:

```
fn f(int x) {
```

and presses Enter, rustyline submits the incomplete input immediately. The
parser then emits an error rather than showing a `...` continuation prompt
and waiting for the closing `}`.

Rustyline supports the `Validator` trait (`validate(&self, ctx) ->
ValidationResult`) for exactly this use case: return `ValidationResult::Incomplete`
when brace nesting is non-zero, causing rustyline to show a secondary prompt
(`.. ` by convention) and continue accumulating input.

## Acceptance criteria

- `repl.rs` implements `rustyline::validate::Validator` for the REPL helper.
- The validator counts `{`/`}`, `(`/`)`, and `[`/`]` nesting depth from
  the accumulated input; returns `ValidationResult::Incomplete` when
  depth > 0.
- The secondary prompt is `".. "` (two dots + space) to distinguish from
  the primary `">> "`.
- An incomplete input that the user abandons with Ctrl-C or Ctrl-D is
  discarded and the REPL returns to the primary prompt.
- Strings and comments are excluded from brace counting (a `{` inside a
  string literal does not increment depth).
- Existing REPL behaviour (single-line evaluation, history persistence
  from RES-238, `:help` / `:examples` commands) is unchanged.
- New unit test in `repl.rs` or `tests/repl_smoke.rs` verifies that
  `validate` returns `Incomplete` for an unclosed `{` and `Valid` for a
  balanced expression.
- Commit: `RES-257: REPL multi-line input via rustyline Validator`.

## Notes

- `rustyline`'s `Validator` trait is in `rustyline::validate`. The REPL
  currently uses `rustyline::DefaultEditor`; switching to a custom
  `Editor<Helper, _>` is required to attach the validator.
- The brace-counting approach is conservative: it may produce false
  "incomplete" signals for syntactically malformed input (e.g.,
  `let x = {` without a closing `}` in a broken program). This is
  acceptable — the user can press Ctrl-C to discard.
- Do NOT implement a full parser-based validator; brace counting is the
  right level of fidelity for an interactive REPL.
- `resilient/src/repl.rs` line 99 already notes "Read input with tab
  completion" — this ticket extends that infrastructure.

## Log

- 2026-04-20 created by analyzer (no rustyline Validator found in repl.rs during code review)
