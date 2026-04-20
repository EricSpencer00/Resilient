---
id: RES-238
title: "REPL history: RESILIENT_HISTORY env-var override and 1000-entry cap not implemented"
state: DONE
priority: P3
goalpost: G11
created: 2026-04-20
owner: executor
Claimed-by: Claude
Closed-by: ed25d70
---

## Summary

RES-224 shipped partial REPL history persistence (load/save via
`rustyline`) but two acceptance criteria from that ticket remain
unimplemented in `resilient/src/repl.rs`:

1. **`RESILIENT_HISTORY` environment-variable override is absent.**
   `EnhancedREPL::with_examples_dir` (lines 41-53) resolves the
   history path only via `HOME`:

   ```rust
   let history_path = match env::var("HOME") {
       Ok(home) => Path::new(&home).join(".resilient_history"),
       Err(_)   => Path::new(".resilient_history").to_path_buf(),
   };
   ```

   The spec requires checking `RESILIENT_HISTORY` first and falling
   back to `~/.resilient_history`. Users who want a non-default
   location (e.g. in CI sandboxes) have no way to override it.

2. **History entry cap is not set.**
   `rustyline::DefaultEditor` defaults to an unbounded (or
   implementation-defined) history size. The spec requires a
   1000-entry cap implemented as a compile-time constant in
   `repl.rs`. Without it, long-lived REPL sessions accumulate
   arbitrarily large history files.

## Acceptance criteria

- In `EnhancedREPL::with_examples_dir`, check `env::var("RESILIENT_HISTORY")`
  first; use its value as the history path if present and non-empty.
  Fall back to `~/.resilient_history` (via `HOME`) otherwise.
- Define `const HISTORY_LIMIT: usize = 1000;` in `repl.rs`.
- Apply the limit to the `rustyline` editor:
  `rl.set_max_history_size(HISTORY_LIMIT)?` (or the equivalent
  `DefaultEditor` config API — check `rustyline 12.x` docs).
- Unit tests (new, not modifying existing tests):
  - `RESILIENT_HISTORY=/tmp/my_hist` → `history_path` resolves
    to `/tmp/my_hist`.
  - When `RESILIENT_HISTORY` is unset, `history_path` resolves to
    `~/.resilient_history` (requires `HOME` to be set, or fallback
    path otherwise).
- `cargo test` remains fully green.
- `cargo clippy --all-targets -- -D warnings` remains clean.
- Commit message: `RES-238: REPL history env-var override and 1000-entry cap`.

## Affected code

- `resilient/src/repl.rs` — `EnhancedREPL::with_examples_dir`
  (lines ~41-53) and `run` (line ~57 where editor is created).

## Notes

- Do **not** modify existing tests — add only new ones.
- `rustyline 12.0.0` exposes `Editor::set_max_history_size`;
  confirm the exact API in the crate docs before coding.
- This is a small, self-contained change with no parser or VM
  involvement.

## Dependencies

- RES-224 (partial predecessor — already shipped the basic load/save).

## Log
- 2026-04-20 created by analyzer (gap between RES-224 spec and repl.rs impl)
