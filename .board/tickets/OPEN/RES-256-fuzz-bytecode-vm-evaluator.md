---
id: RES-256
title: "Fuzz harness for the bytecode VM evaluator"
state: OPEN
priority: P2
goalpost: G9
created: 2026-04-20
owner: executor
---

## Summary

The fuzz suite (`fuzz/fuzz_targets/`) covers only the lexer and parser.
The bytecode VM (`resilient/src/vm.rs`) and the compiler that feeds it
(`resilient/src/compiler.rs`) have no fuzz coverage. Malformed or
adversarially-crafted programs that survive parsing could trigger panics
or undefined behaviour in the VM, which is especially concerning for a
language targeting safety-critical embedded systems.

## Current fuzz targets

```
fuzz/fuzz_targets/lex.rs   — exercises the hand-rolled lexer
fuzz/fuzz_targets/parse.rs — exercises the parser
```

## Acceptance criteria

- New fuzz target: `fuzz/fuzz_targets/eval.rs`.
  - Input: arbitrary bytes fed through the lexer → parser → compiler → VM.
  - The harness must NOT panic at any input. All error paths must return
    a `Result::Err` through the typed error infrastructure.
  - The harness gates on `resilient::parse` followed by
    `resilient::compiler::compile` followed by `resilient::vm::run`, and
    simply returns if any step returns an error.
  - Timeout: the harness must complete within 1 second per input (guard
    against infinite loops via the VM's existing step-count limit or a new
    `--max-steps` ceiling).
- The Cargo.toml in `fuzz/` is updated to declare the new target.
- `cargo fuzz run eval -- -max_total_time=30` completes without crashes
  on the developer's machine (30-second smoke run).
- No panics are introduced in the compiler or VM as a side effect.
- Commit: `RES-256: fuzz harness for bytecode VM evaluator`.

## Notes

- The VM's `run` function is in `resilient/src/vm.rs`. It currently panics
  in a few places for internal invariant violations — those must be
  converted to `Err(VmError::...)` before this harness can safely run.
  File a sub-task if the conversion is large.
- The step-count limit prevents infinite loops from hanging the fuzzer. If
  the VM doesn't already have one, add a `max_steps: Option<u64>` to the
  `run` call signature.
- The compiler's `dead_code` suppression (`#![allow(dead_code)]`) can stay;
  it doesn't affect fuzz coverage.
- Run the fuzz target through CI with a short `-max_total_time` to catch
  regressions (see RES-202 for the perf-CI gate pattern).

## Log

- 2026-04-20 created by analyzer (only lex.rs and parse.rs exist in fuzz/fuzz_targets/)
