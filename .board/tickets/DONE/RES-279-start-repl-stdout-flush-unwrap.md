---
id: RES-279
title: "main.rs: start_repl() has io::stdout().flush().unwrap() in production REPL clear handler"
state: DONE
priority: P4
goalpost: G11
created: 2026-04-20
owner: executor
Claimed-by: Claude Sonnet 4.6
Closed-by: fca05a0
---

## Summary

`resilient/src/main.rs` line 8056 contains an `.unwrap()` in the production
`start_repl()` function (the legacy/basic REPL entry point), specifically in
the "clear" command handler:

```rust
"clear" => {
    print!("\x1B[2J\x1B[1;1H"); // ANSI escape code to clear screen
    io::stdout().flush().unwrap();
    continue;
}
```

`io::stdout().flush()` returns `io::Result<()>` which can fail (e.g., broken
pipe, redirected stdout to a file on a full disk). The `.unwrap()` would then
panic, violating CLAUDE.md's no-panic rule for production code.

This is analogous to the defect fixed by RES-273, which patched the same
pattern in `repl.rs` (the `EnhancedREPL` path), but the legacy `start_repl()`
in `main.rs` was left unfixed.

## Affected code

`resilient/src/main.rs` — `start_repl()` function, approximately line 8056:

```rust
fn start_repl() -> RustylineResult<()> {
    // ... (starts at line 7994)
    "clear" => {
        print!("\x1B[2J\x1B[1;1H"); // ANSI escape code to clear screen
        io::stdout().flush().unwrap();   // ← panic risk
        continue;
    }
```

## Fix

Replace `.unwrap()` with a `let _ = ...` to silently discard flush errors
(acceptable for a terminal-clear operation where flushing is best-effort):

```rust
"clear" => {
    print!("\x1B[2J\x1B[1;1H");
    let _ = io::stdout().flush();
    continue;
}
```

## Acceptance criteria

- Line ~8056 in `main.rs` is changed from
  `io::stdout().flush().unwrap();` to `let _ = io::stdout().flush();`.
- No behavioural change for normal interactive REPL use.
- `cargo test` passes with 0 failures.
- `cargo clippy --all-targets -- -D warnings` clean.
- Commit: `RES-279: start_repl clear handler — drop flush().unwrap() for let _ = flush()`.

## Notes

- This is the same class of defect as RES-273 (repl.rs) and RES-261
  (formatter). Priority P4 because stdout flush failures are extremely rare
  in interactive REPL usage.
- Do NOT modify any tests.

## Log

- 2026-04-20 created by analyzer (main.rs start_repl() line 8056 has
  io::stdout().flush().unwrap() in the "clear" command handler of the
  production REPL; RES-273 fixed the same pattern in repl.rs but missed
  this second occurrence in main.rs)
