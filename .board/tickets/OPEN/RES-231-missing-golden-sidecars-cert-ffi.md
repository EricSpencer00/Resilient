---
id: RES-231
title: "Missing `.expected.txt` golden sidecars for `cert_demo.res` and `ffi_libm.res`"
state: OPEN
priority: P3
goalpost: testing
created: 2026-04-20
owner: executor
---

## Summary
`cargo test -- --ignored missing_expected_files_are_intentional`
reports two examples with no `.expected.txt` golden sidecar:

```
2 example(s) have no .expected.txt sidecar:
  cert_demo.res
  ffi_libm.res
```

Both are non-interactive examples added after RES-006 closed
the original golden sweep. Neither has a `.interactive` marker
file (which would intentionally exempt them from the audit).

## Per-example analysis

### `cert_demo.res`
- Runs successfully (`resilient examples/cert_demo.res` exits 0).
- Output is `42\nProgram executed successfully`.
- **Blocker**: the binary prints a `seed=NNNN` line to stdout before
  the program output. This line changes on every run (it is a random
  RNG seed used internally). The golden test compares stdout
  byte-for-byte (after trimming trailing whitespace), so a fixed
  `.expected.txt` would fail non-deterministically.
- Resolution options (pick one):
  a. Make the seed output go to stderr instead of stdout.
  b. Gate seed output behind a `--verbose` / `--debug` flag.
  c. Suppress it entirely (the seed is useful only for reproducing
     fuzz failures; it doesn't belong in normal run output).
  d. Add line-filtering to the golden test harness (invasive).
  Option (c) or (a) is preferred — it also fixes all other golden
  files that would otherwise carry the seed in their expected output.

### `ffi_libm.res`
- **Blocked by RES-229**: the example currently fails to parse
  because `parse_extern_block` has a cursor desync bug. Fix RES-229
  first.
- After RES-229: the example runs correctly only on Linux (where
  `libm.so.6` is available). On macOS the library name differs
  (`libm.dylib`). The smoke test (`examples_smoke.rs`) already
  gates this with `#[cfg(all(feature = "ffi", target_os = "linux"))]`.
- Options for a golden file:
  a. Add a `.interactive` marker so the golden audit ignores it
     (matches the fact it is platform-specific).
  b. Add a platform-conditional `.expected.txt` (complex).
  Option (a) is simplest — mark it as a platform demo and add a
  comment in the file noting the Linux-only constraint.

## Acceptance criteria
- `cargo test -- --ignored missing_expected_files_are_intentional`
  reports **0** examples with missing sidecars.
- For `cert_demo.res`: the `seed=` line is no longer printed to
  stdout on normal runs (or an appropriate mitigation from the
  options above is applied), AND `cert_demo.expected.txt` is added
  and passes `golden_outputs_match`.
- For `ffi_libm.res`: either a `.interactive` marker is added (and
  the file documents the Linux/macOS requirement), or a proper
  platform-conditional golden is provided.
- `cargo test` remains fully green.
- Commit message: `RES-231: add golden sidecars for cert_demo and ffi_libm`.

## Dependencies
- RES-229 must land before `ffi_libm.res` can be addressed.

## Notes
- Do **not** modify existing tests or golden files.
- If suppressing the `seed=` line, check all existing golden files to
  confirm none accidentally captured the seed (they were all created
  after the seed was added, so they should not contain it, but verify).

## Log
- 2026-04-20 created by analyzer
