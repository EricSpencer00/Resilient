---
id: RES-026
title: REPL loads examples dir via argument
state: DONE
priority: P2
goalpost: G11
created: 2026-04-16
owner: executor
---

## Summary
The REPL's `examples` command (see `EnhancedREPL::show_examples` in
`resilient/src/repl.rs:194`) currently prints three hardcoded snippets
inline. That made sense before we had any real example files; now
that `resilient/examples/` ships ~10 substantive `.rs` files, the
REPL should be able to enumerate them directly. This ticket adds a
`--examples-dir <DIR>` driver flag that wires the REPL to a real
directory; in interactive mode `examples` then lists the files there
and `examples <name>` prints the contents of one.

## Acceptance criteria
- New driver flag `--examples-dir <DIR>` (also accepts `=DIR`).
  Sets the dir; default behavior (no flag) preserves the existing
  hardcoded snippets so we don't regress.
- `EnhancedREPL::new()` is replaced by `EnhancedREPL::with_examples_dir(Option<PathBuf>)` (or a builder method); `repl.rs` stores the optional dir on the struct.
- `examples` command, when a dir is set, prints the sorted list of `.rs` files in that directory (just basenames). Otherwise falls back to the legacy hardcoded snippets.
- New `examples <name>` subcommand: when a dir is set and `<name>` (with or without `.rs` suffix) resolves to a file under it, print the file's contents to stdout. Unknown name → red `examples: no such file '<name>' in <dir>`.
- `help` text updated to mention the `<name>` form.
- Path traversal guard: reject `<name>` containing `/` or `..` with `examples: name must be a single basename, not a path`.
- New unit test in `repl.rs`'s `#[cfg(test)] mod tests` (create the module if absent) that constructs an `EnhancedREPL::with_examples_dir(Some(tmp))` against a temp dir holding `foo.rs`, and asserts `examples` produces a listing containing `foo.rs`. Use a private helper that returns the listing as `String` rather than printing, so the test doesn't fight stdout capture.
- `cargo build`, `cargo test`, `cargo clippy -- -D warnings` all pass.
- Commit message: `RES-026: REPL examples command honors --examples-dir`.

## Notes
- Driver lives at `resilient/src/main.rs:3678` (REPL launch path).
- Snippet renderer at `resilient/src/repl.rs:194`.
- Don't pull in any new dependency for tempdir — `std::env::temp_dir().join(format!("res026_{}", std::process::id()))` is enough; clean up at the end of the test.
- Integration test (running the actual binary in `--examples-dir` mode) is OUT of scope here — the REPL is interactive and would need a pty harness. The unit test on the listing helper is the proof.

## Log
- 2026-04-16 created by manager
- 2026-04-17 acceptance criteria filled in by manager (orchestrator pass)
- 2026-04-17 executor landed:
  - `EnhancedREPL::with_examples_dir(Option<PathBuf>)` constructor;
    `new()` preserved as a thin legacy wrapper.
  - Driver: `--examples-dir <DIR>` (also `=DIR`); the REPL launch path
    threads it through.
  - `examples` command now does dynamic listing when the dir is set,
    fallback to the legacy hardcoded snippets when it isn't.
  - `examples <name>` subcommand prints one file's contents (with a
    `cyan --- {path} ---` header). Path traversal guard rejects names
    containing `/` or `..`.
  - Pure helper `EnhancedREPL::list_examples_in(&Path) -> Result<String, String>`
    returns the listing as a String — testable without stdout capture
    fights.
  - `help` text branches on whether `--examples-dir` is set so the
    user always sees the right surface.
- 2026-04-17 tests: 4 new unit tests in a new `repl::tests` module:
  sorted basenames, non-`.rs` filtering, missing-dir clean error,
  constructor wiring (both `with_examples_dir` and `new`). Each test
  isolates with `temp_dir().join(format!("res_026_{}_{}", label, pid))`
  and cleans up on exit.
- 2026-04-17 verification: 166 tests default / 175 with `--features z3`.
  `cargo build`, `cargo clippy -- -D warnings` clean both ways.
