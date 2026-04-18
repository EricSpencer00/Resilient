---
id: RES-145
title: String builtins: replace / to_upper / to_lower / format
state: DONE
priority: P3
goalpost: G11
created: 2026-04-17
owner: executor
---

## Summary
RES-043 added split/trim/contains/substring. The next ergonomic gap
is case conversion, substring replacement, and a minimal format
string function. Lock down a small, predictable surface.

## Acceptance criteria
- `replace(s: String, from: String, to: String) -> String` — all
  occurrences, left-to-right, non-overlapping.
- `to_upper(s: String) -> String` and `to_lower(s: String) ->
  String` — ASCII-only behavior to avoid locale surprises.
  Document the ASCII-only semantics.
- `format(fmt: String, args: Array<?>) -> String` —
  `{}` placeholders consume args in order; `{{` and `}}` escape.
  Unknown / mismatched args → runtime error with span.
- Unit tests: one success + one error path per builtin.
- Typechecker signatures added to the builtin table in
  `typechecker.rs`.
- Commit message: `RES-145: string builtins — replace/upper/lower/format`.

## Notes
- `format` is NOT printf — no width / precision / type specifiers.
  Keep the grammar to `{}` for the MVP; expand in a follow-up if
  needed.
- All functions return new Strings; don't mutate in place — our
  runtime has no mutable-String concept yet.

## Log
- 2026-04-17 created by manager
- 2026-04-17 claimed by executor
- 2026-04-17 done by executor

## Resolution
- `resilient/src/main.rs`:
  - `builtin_to_upper` / `builtin_to_lower` switched from
    Unicode `to_uppercase` / `to_lowercase` to ASCII-only
    `to_ascii_uppercase` / `to_ascii_lowercase`. Non-ASCII
    code points pass through untouched (e.g. `á` stays `á`).
    Doc comments spell out the locale-avoidance rationale.
  - New `builtin_replace(s, from, to)` — uses `str::replace`
    for all-occurrences, left-to-right, non-overlapping
    substitution. Hard-errors on empty `from` (stdlib's
    behaviour of splicing between every char is almost
    always a bug).
  - New `builtin_format(fmt, args: Array)` — MVP string
    interpolator. Grammar: `{}` consumes next arg in order,
    `{{` / `}}` escape to literal `{` / `}`. Rejects
    unmatched `{` / `}`, printf-style specifiers like
    `{:04}`, and arity mismatches (too few / too many
    args) with clean per-case diagnostics.
  - All three new builtins wired into the `BUILTINS` table.
- `resilient/src/typechecker.rs`: `replace` registered as
  `fn(String, String, String) -> String`; `format` registered
  as `fn(String, Array) -> String` (the `Array<?>` signature
  from the ticket — our `Type::Array` has no element
  parameter yet).
- Deviations from ticket: the acceptance criterion says
  "Unknown / mismatched args → runtime error with span". The
  builtin layer produces the runtime error string; the span
  attachment is done at the interpreter call-site level
  (tree walker's existing error path), not inside the
  builtin — same precedent as every other RES-0xx builtin
  error (e.g. `file_read`'s IO errors). No builtin currently
  carries a span argument; adding that plumbing would be its
  own ticket.
- Unit tests (in `main.rs` test module, 13 new):
  - `replace_substitutes_all_occurrences` — happy path
  - `replace_empty_from_errors` — error path
  - `to_upper_is_ascii_only` — `ábc xYz` → `áBC XYZ`
  - `to_upper_rejects_non_string` — error path
  - `to_lower_is_ascii_only` — `ÁBC XyZ` → `Ábc xyz`
  - `to_lower_rejects_non_string` — error path
  - `format_interpolates_placeholders_in_order` — happy path
  - `format_escapes_double_braces` — `{{` / `}}` escapes
  - `format_errors_on_too_few_args`
  - `format_errors_on_too_many_args`
  - `format_errors_on_unmatched_close_brace`
  - `format_errors_on_unsupported_specifier` — rejects
    `{:04}` cleanly
  - `format_rejects_non_array_second_arg`
- Verification:
  - `cargo test --locked` — 347 passed (was 334 before RES-145)
  - `cargo test --locked --features logos-lexer` — 348 passed
  - `cargo clippy --locked --features logos-lexer,z3 --tests
    -- -D warnings` — clean
