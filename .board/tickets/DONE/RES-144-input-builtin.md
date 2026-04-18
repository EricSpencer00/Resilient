---
id: RES-144
title: `input(prompt) -> String` builtin reads a line from stdin
state: DONE
priority: P3
goalpost: G11
created: 2026-04-17
owner: executor
---

## Summary
Teaching examples need interactivity — "enter your name" style. A
single-line read from stdin, optional prompt printed first, is the
minimal version.

## Acceptance criteria
- Builtin `input(prompt: String) -> String`. `input("")` is the
  prompt-less form.
- Reads up to the first `\n`, strips trailing `\r\n`/`\n`.
- EOF before any data returns `""`.
- Only in std build; the no_std runtime doesn't get this.
- Unit test uses a stubbed stdin (via
  `std::io::Cursor` injected through a small trait) to verify
  behavior without blocking the test on real stdin.
- `examples/interactive_greeter.rs` — `cargo run` demo; no golden
  (it's interactive).
- Commit message: `RES-144: input() builtin`.

## Notes
- Don't read from stdin at build time — the golden-test framework
  walks examples/ and runs them; this example must be skipped by
  the harness. Extend the harness to look for an `.interactive`
  sidecar file that marks an example as "don't exec in CI".
- Returning `""` on EOF rather than erroring is deliberate: lets
  loops like `while (input("> ") != "quit")` terminate on
  ctrl-D.

## Log
- 2026-04-17 created by manager
- 2026-04-17 claimed by executor
- 2026-04-17 done by executor

## Resolution
- `resilient/src/main.rs`:
  - New `builtin_input(args)` — reads a single line from stdin via
    `std::io::stdin().lock()` and returns the contents with any
    trailing `\r\n` or `\n` stripped. Empty prompt skips the print;
    non-empty prompt is `print!`'d and `flush`'d before the read
    blocks so interactive shells see it immediately.
  - Internal helper `do_input<R: BufRead>(reader, prompt)` factored
    out for testing. `BufRead` is the "small trait" the ticket
    mentions — `std::io::Cursor` implements it, so tests can feed
    synthetic stdin without blocking.
  - EOF before any bytes is returned as `Value::String("")` (not an
    error) so `while input("> ") != "quit"` exits cleanly on
    ctrl-D per the ticket's Notes.
  - Registered in the `BUILTINS` table with signature `input(p)`.
- `resilient/src/typechecker.rs`: `input` registered in the prelude
  env as `fn(String) -> String`.
- `resilient/tests/examples_golden.rs`:
  - New `is_interactive(path)` helper checks for a sibling
    `<stem>.interactive` marker file.
  - `golden_outputs_match` skips interactive examples entirely.
  - `missing_expected_files_are_intentional` exempts them from the
    "missing `.expected.txt`" audit.
- `resilient/examples/interactive_greeter.rs` — demo program that
  prompts for a name and greets it, handling the empty / EOF case.
  Sibling `interactive_greeter.interactive` marker keeps it out
  of CI.
- Deviations: the ticket says "via a small trait" — I used stdlib
  `std::io::BufRead` rather than defining a custom trait. It
  satisfies the same injection requirement (tests drive
  `do_input` with `Cursor`) without adding an abstraction that
  duplicates what the ecosystem already provides.
- Unit tests (in `main.rs` test module):
  - `do_input_reads_single_line_and_strips_newline`
  - `do_input_strips_crlf_line_endings` — Windows CRLF round-trip
  - `do_input_returns_empty_string_on_eof`
  - `do_input_reads_only_first_line_if_multiple_present`
  - `do_input_line_without_trailing_newline_still_returned`
  - `builtin_input_rejects_non_string_prompt`
  - `builtin_input_rejects_wrong_arity`
- End-to-end smoke (manual):
  - `echo "alice" | cargo run -- examples/interactive_greeter.rs`
    → `What is your name? Hello, alice!`
  - `echo -n "" | cargo run -- examples/interactive_greeter.rs`
    → `What is your name? (no name given — goodbye!)`
- Verification:
  - `cargo test --locked` — 334 passed (was 327 before RES-144)
  - `cargo test --locked --features logos-lexer` — 335 passed
  - `cargo clippy --locked --features logos-lexer,z3 --tests -- -D
    warnings` — clean
  - `cargo test --locked -- --ignored missing_expected_files_are_intentional`
    still names only pre-existing `cert_demo.rs` (interactive
    greeter correctly exempted).
