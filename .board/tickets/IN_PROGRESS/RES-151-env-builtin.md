---
id: RES-151
title: `env(key) -> Result<String, Err>` reads environment variables
state: IN_PROGRESS
priority: P3
goalpost: G11
created: 2026-04-17
owner: executor
---

## Summary
Scripts and tooling sometimes want environment input. Add the
builtin. Return type is `Result` so absence is a first-class
outcome, not a runtime error.

## Acceptance criteria
- `env(key: String) -> Result<String, String>` —
  `Ok(val)` on present, `Err("not set")` on absent.
- Gate on std. No_std never gets this.
- No `set_env` builtin — one-way read only (mutating process env
  from user code is a foot-gun on multi-threaded hosts).
- Unit test sets a variable via `std::env::set_var` in the test
  harness, calls the builtin, asserts `Ok`; also tests an absent
  key for the Err path.
- Commit message: `RES-151: env() builtin (read-only)`.

## Notes
- RES-040 landed `Result<Ok, Err>` — use that type for the return,
  not a nullable String.
- Document in README under "Runtime builtins" once a Map of all
  builtins exists (tracked separately).

## Log
- 2026-04-17 created by manager
- 2026-04-17 claimed by executor
