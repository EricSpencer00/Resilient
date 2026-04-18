---
id: RES-151
title: `env(key) -> Result<String, Err>` reads environment variables
state: DONE
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
- 2026-04-17 done by executor

## Resolution
- `resilient/src/main.rs`:
  - New `builtin_env(args)` — reads `std::env::var(key)` and
    surfaces three outcomes:
    - present → `Ok(Value::String(val))`
    - `VarError::NotPresent` → `Err("not set")`
    - `VarError::NotUnicode(_)` → `Err("invalid utf-8")`
    The non-UTF-8 branch is additive to the ticket's
    Ok/NotSet dichotomy; it keeps the "absence isn't a runtime
    halt" spirit by not panicking on invalid UTF-8 bytes.
  - Registered in `BUILTINS` as `("env", builtin_env)`.
  - Deliberately no `set_env` — ticket says one-way, and
    `std::env::set_var` is `unsafe` in recent Rust editions
    for exactly the multi-threaded-host footgun the ticket
    calls out.
- `resilient/src/typechecker.rs`: `env` registered as
  `fn(String) -> Result` — using RES-040's first-class
  `Type::Result` per the ticket's Notes.
- Deviations: none. std-only; no no_std wiring.
- Unit tests (4 new):
  - `env_returns_ok_for_set_variable` — sets a pid-unique var,
    calls the builtin, asserts `Ok("hello")`. Wraps the
    `unsafe { set_var }` / `unsafe { remove_var }` calls and
    serializes via `ENV_TEST_LOCK` since Rust now marks env
    mutation unsafe for cross-thread safety.
  - `env_returns_err_not_set_for_missing_variable` — removes
    the unique key, asserts `Err("not set")`.
  - `env_rejects_non_string_key` — Int input → clean error.
  - `env_rejects_wrong_arity` — zero-arg and two-arg cases.
- Verification:
  - `cargo test --locked` — 390 passed (was 386 before RES-151)
  - `cargo test --locked --features logos-lexer` — 391 passed
  - `cargo clippy --locked --features logos-lexer,z3 --tests
    -- -D warnings` — clean
