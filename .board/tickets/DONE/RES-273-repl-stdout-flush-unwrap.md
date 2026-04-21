---
id: RES-273
title: "repl.rs: io::stdout().flush().unwrap() in production REPL clear handler"
state: IN_PROGRESS
priority: P4
goalpost: G11
created: 2026-04-20
owner: executor
Claimed-by: Claude Sonnet 4.6
---

## Summary

`resilient/src/repl.rs` line 157 contains an `.unwrap()` in production REPL
code (the "clear" command handler):

```rust
"clear" => {
    print!("\x1B[2J\x1B[1;1H"); // ANSI escape code to clear screen
    io::stdout().flush().unwrap();
    return;
}
```

`io::stdout().flush()` returns `io::Result<()>` which can theoretically fail
(e.g., broken pipe, redirected stdout to a file on a full disk). The
`.unwrap()` would then panic, violating CLAUDE.md's no-panic rule for
library/production code.

The clean replacement is to silently ignore the flush error (acceptable
for a terminal-clear operation where flushing is best-effort) or log it
to stderr:

```rust
let _ = io::stdout().flush(); // best-effort; ignore flush errors
```

## Acceptance criteria

- Line 157 in `repl.rs` is changed from
  `io::stdout().flush().unwrap();` to
  `let _ = io::stdout().flush();`.
- No behavioural change for normal interactive use.
- `cargo test` passes with 0 failures.
- `cargo clippy --all-targets -- -D warnings` clean.
- Commit: `RES-273: repl clear handler — drop flush().unwrap() for let _ = flush()`.

## Notes

- This is the same class of defect as RES-261 / RES-264 (infallible-in-
  practice unwrap in production code). Priority P4 because stdout flush
  failures are extremely rare in interactive REPL usage.
- Do NOT change the `println!` / `print!` macro calls in the same method —
  those use `io::Write` internally and also silently drop I/O errors, which
  is the standard Rust convention for stdout.

## Log

- 2026-04-20 created by analyzer (repl.rs line 157 has io::stdout().flush().unwrap()
  in the production "clear" command handler; violates no-panic rule in CLAUDE.md)
