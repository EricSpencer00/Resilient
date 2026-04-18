---
id: RES-201
title: cargo-fuzz harness for the parser (no panics on any input)
state: IN_PROGRESS
priority: P3
goalpost: testing
created: 2026-04-17
owner: executor
---

## Summary
Companion to RES-111's lexer fuzz: the parser must also never
panic on any byte sequence. After RES-016 killed the known
panics, a fuzzer proves the rest.

## Acceptance criteria
- New fuzz target `fuzz/fuzz_targets/parse.rs`: take arbitrary
  bytes, filter to UTF-8, feed through `Parser::new(...).parse()`.
- Invariant: no panics, no infinite loops (250ms timeout).
- Any crash reduces to a unit test + fix in the same PR.
- The existing `fuzz.yml` workflow (from RES-111) is extended to
  run both targets — 30 seconds each on manual dispatch.
- Commit message: `RES-201: parser fuzz target`.

## Notes
- Parser recovery (RES-016) means even malformed input should
  produce a Diagnostic vec rather than a panic. This fuzz target
  pins that.
- Optional: have the target also invoke the typechecker on the
  AST — but only on parse-success, to avoid OOM from the checker
  on pathological input.

## Log
- 2026-04-17 created by manager
- 2026-04-17 claimed by executor
