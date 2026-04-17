---
id: RES-006
title: Golden tests for all example programs
state: OPEN
priority: P2
goalpost: G2
created: 2026-04-16
owner: executor
---

## Summary
We need an automated safety net that runs every program in
`resilient/examples/` and asserts its output matches a committed
expected-output file. This catches regressions from every future
language change.

Depends on RES-002 (test harness) and RES-003 (`println` builtin).

## Acceptance criteria
- `resilient/tests/examples.rs` spawns the compiled binary against
  each `.rs` file in `resilient/examples/` and compares stdout to a
  sibling `.expected.txt` file
- At least `hello.rs`, `minimal.rs` have golden output files; others
  may be marked with a `#[ignore]` attribute if they still fail
  legitimately (document each ignore with a ticket reference)
- Running `cargo test --test examples` passes
- `run_examples.sh` is deprecated or updated to delegate to
  `cargo test --test examples`

## Notes
- Spawn the binary via `std::process::Command::new(env!("CARGO_BIN_EXE_resilient"))`.
- Store expected outputs as siblings: `hello.rs` → `hello.expected.txt`.
- Trim trailing whitespace before comparing to avoid newline flakiness.

## Log
- 2026-04-16 created by session 0
