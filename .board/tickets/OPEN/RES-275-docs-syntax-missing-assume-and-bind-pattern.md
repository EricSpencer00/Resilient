---
id: RES-275
title: "docs/syntax.md and language-reference.md missing assume() and name @ pattern"
state: OPEN
priority: P3
goalpost: G12
created: 2026-04-20
owner: executor
---

## Summary

Two language features shipped in PRs #48 and #49 (commits `6ada8e3` and
`94595a5`) are absent from the public documentation:

1. **`assume(expr)`** — assertion hint to the runtime and (RES-235) the SMT
   verifier. Implemented in RES-133a. Not mentioned anywhere in
   `docs/syntax.md` or `docs/language-reference.md`.

2. **`name @ inner`** bind patterns — a match arm can bind the whole matched
   value to `name` while also testing against `inner`. Implemented in
   RES-161a. Not mentioned anywhere in the documentation.

Both features are exercised by tests and golden sidecars (in open PRs), but a
user reading the docs has no way to discover them.

## Affected files

- `docs/syntax.md` — 432 lines; contains zero occurrences of `assume` or `@`.
- `docs/language-reference.md` — does not describe these constructs.

## Acceptance criteria

### syntax.md

Add a **"Runtime assertions"** subsection (or extend the existing assertions
section) documenting `assume()`:

```
assume(condition);
```

- States that `assume` is a runtime no-op when the condition holds; traps
  with a message when it doesn't.
- Notes that the SMT verifier (when enabled) treats `assume` as an axiom
  (see RES-235 for verifier integration status).
- Provide a one-line example:
  ```
  assume(x > 0);  // asserts x is positive; verifier treats as axiom
  ```

Add a **"Bind patterns"** subsection inside the "Match expressions" section
documenting `name @ inner`:

```
match value {
    n @ 1..=10 => { /* n is bound to value, 1..=10 is the test */ }
    _ => {}
}
```

- Explains that `name` is bound to the whole matched value even if `inner`
  further constrains it.
- Shows the range and wildcard combinations.

### language-reference.md

- Add `assume` to the keyword / built-in table.
- Add `@` to the operator / pattern table.
- Cross-reference the syntax.md sections.

### Tests / CI

- `docs/verify_tutorial_snippets.sh` (if it covers syntax.md snippets) must
  continue to pass.
- No source code changes required.

- Commit: `RES-275: docs — add assume() and name @ bind-pattern to syntax and reference`.

## Notes

- Do not modify any existing test or golden-output file.
- The `assume` runtime behaviour is already implemented; only the documentation
  is missing.
- The SMT verifier integration for `assume` (RES-235) is still open; the doc
  should note it as a "planned enhancement" rather than a current feature.

## Log

- 2026-04-20 created by analyzer (docs/syntax.md contains zero occurrences of
  `assume` or `@`; both features shipped in #48/#49 but were never documented)
