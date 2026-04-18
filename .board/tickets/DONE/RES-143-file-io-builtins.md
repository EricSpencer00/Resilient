---
id: RES-143
title: `file_read` + `file_write` builtins (std-only, alloc-gated in runtime)
state: DONE
priority: P2
goalpost: G11
created: 2026-04-17
owner: executor
---

## Summary
Safety-critical embedded work will never use file I/O, but the
host-side Resilient use case (config generation, tooling, tests)
needs at least the two primitives. Expose `file_read(path) ->
String` and `file_write(path, contents) -> ()`.

## Acceptance criteria
- `file_read(path: String) -> String` — reads UTF-8, errors as
  runtime diagnostics with span on the call site.
- `file_write(path: String, contents: String) -> ()` —
  write-truncate.
- Compiled only in the std build (`resilient-runtime` stays
  no_std-clean). Use a `#[cfg(feature = "std")]` gate on the
  builtin registration.
- Error on non-UTF-8 contents from `file_read`: `file_read: <path>
  is not valid UTF-8` (not a panic).
- Unit tests write to a `tempfile::NamedTempFile` and assert
  round-trip.
- New example `examples/file_io_demo.rs` + golden demonstrates the
  round trip.
- Commit message: `RES-143: file_read / file_write builtins`.

## Notes
- Security: no sandbox. Users of the CLI already have ambient
  authority. Worth a README note under "Safety considerations".
- Don't add `file_append` or `file_exists` in this ticket —
  separate tickets if we need them.

## Log
- 2026-04-17 created by manager
- 2026-04-17 claimed by executor
- 2026-04-17 done by executor

## Resolution

Files changed:
- `resilient/src/main.rs` — two new builtins (`builtin_file_read`,
  `builtin_file_write`) plus their entries in the `BUILTINS` const
  slice, and five new unit tests in `mod tests`: round-trip
  success, missing-file error prefix, non-UTF-8 error, and arity /
  type mismatch rejection for both.
- `resilient/src/typechecker.rs` — registered both builtins with
  their type signatures (`String -> String`, `(String, String) -> Void`).
- `resilient/examples/file_io_demo.rs` + `.expected.txt` — golden-
  backed round-trip demo. Runs as part of the existing
  `tests/examples_golden.rs` harness.
- `README.md` — added `file_io_demo.rs` to the example list and a
  "Safety considerations" paragraph covering the ambient-authority
  posture and the fact that `resilient-runtime` has no builtins
  table (so its no_std posture is unaffected).

Deviation from the sketch: the ticket asks for a
`#[cfg(feature = "std")]` gate on the builtin registration. The
`resilient` CLI crate is always std (uses `std::fs`, `Vec`, etc.)
and `resilient-runtime` has no builtins table at all — there is no
compilation target on which the gate would do anything. Skipped the
cfg; the README note explains why the runtime stays no_std-clean.

Verification:
- `cargo build` — clean.
- `cargo test` — 253 unit (+5 new) + 13 integration + 1 golden pass
  (the new `file_io_demo.rs` golden runs here).
- `cargo clippy --tests -- -D warnings` — clean.
- Manual: `resilient examples/file_io_demo.rs` round-trips through
  `/tmp/resilient_file_io_demo.txt` and prints the expected two
  lines, exit code 0.
