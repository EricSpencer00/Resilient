---
id: RES-234
title: "Add `.expected.txt` golden sidecar for `name @ pattern` bind examples"
state: DONE
Claimed-by: Claude Sonnet 4.6
priority: P3
goalpost: testing
created: 2026-04-20
owner: executor
---

## Summary

`Pattern::Bind` (`name @ inner`) was shipped in RES-161a with only unit
tests inside `#[cfg(test)]`. CLAUDE.md requires: _"New language features:
add an `.expected.txt` golden sidecar in `resilient/examples/`."_ No
example file or golden sidecar was added for the `@` bind-pattern.

## Context

The unit tests in `src/main.rs` (section starting at line 14468) verify
the interpreter behaviour, but a user browsing `examples/` has no
runnable demonstration of the feature. The golden harness also provides
CI regression coverage that unit tests alone cannot catch (e.g. a
formatting regression or a CLI change that breaks the feature end-to-end).

## Acceptance criteria

- Add `resilient/examples/bind_pattern_demo.res` demonstrating:
  - `name @ _` — bind the whole value unconditionally.
  - `name @ <literal>` — bind only on a specific value, fall through
    otherwise.
  - A guard that uses the bound name: `name @ _ if name > 0 => ...`.
  - `println` output that proves the correct arm was taken and the bound
    variable holds the expected value.
- Add `resilient/examples/bind_pattern_demo.expected.txt` capturing the
  exact stdout produced by `resilient examples/bind_pattern_demo.res`.
- `cargo test` remains fully green (including `golden_outputs_match`).
- `cargo test -- --ignored missing_expected_files_are_intentional`
  reports the same count as before (the new file has a sidecar).
- Commit message: `RES-234: golden sidecar example for name @ pattern bind`.

## Notes

- Keep the example to < 20 lines; a single `fn main` with three `match`
  expressions is sufficient.
- Do **not** modify existing tests or golden files.
- RES-161b (struct-destructure bind) and RES-161c (tuple bind) are
  separate follow-up tickets; this example should stay within the
  RES-161a scope (literal / identifier / wildcard / or inner patterns).

## Log
- 2026-04-20 created by analyzer
