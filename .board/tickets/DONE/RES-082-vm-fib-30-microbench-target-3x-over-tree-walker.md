---
id: RES-082
title: VM fib(30) microbench vs tree walker
state: DONE
priority: P2
goalpost: G15
created: 2026-04-17
owner: executor
---

## Summary
The RES-076 ticket originally promised a **3× speedup on fib(30)**
as the sign-off measurement for the bytecode VM track. RES-083
(control flow) unblocked fib; this ticket runs the bench and
documents the number.

If the 3× target isn't met, don't fudge — record the real ratio,
investigate the hottest VM op with `cargo flamegraph` or similar,
and file a follow-up for the missing perf work. Honest numbers
beat aspirational ones.

## Acceptance criteria
- New file `benchmarks/fib/fib_vm.rs` that is VM-compatible: NO
  `requires` clauses (contracts aren't bytecode-lowered), NO
  `println` calls (builtins aren't a VM op yet). The top-level is
  just `fib(25);` — the bare call's return value leaks to the stack
  and the driver prints it.
  - Pick the same `n` the existing `fib.rs` uses (currently 25,
    not 30 — the ticket's "fib(30)" was aspirational, but matching
    the existing bench keeps the comparison apples-to-apples).
- `benchmarks/run.sh` gains a `Resilient (VM)` row in the fib bench:
  `$RES --vm benchmarks/fib/fib_vm.rs`.
- Run `./benchmarks/run.sh` and update `benchmarks/RESULTS.md` with
  the new row. Add a one-paragraph note directly beneath the fib
  table recording the VM-vs-interp ratio and a sentence on whether
  the 3× target was met.
- `cargo build`, `cargo test`, `cargo clippy -- -D warnings` all
  pass (this ticket only touches benchmarks/ — no source changes).
- Commit message: `RES-082: fib(25) microbench — VM vs tree walker`.

## Notes
- Hyperfine is already in the dev toolchain (see `run.sh`); don't
  add a new dep.
- The VM runs `Resilient + typecheck + compile + run`. For an
  apples-to-apples compare we DO want that whole pipeline timed
  (that's what users experience). So both rows run the release
  binary end-to-end.
- **If the VM loses to the tree walker**: ship the numbers anyway
  and file a follow-up ticket documenting what the hot path is
  (likely Value cloning on every LoadLocal).
- The 3× target was a hand-wavy goal set before the VM was
  implemented. A stack VM with Value clones on every Load may
  realistically only get 1.5-2×. Record the real number.

## Log
- 2026-04-17 created by manager
- 2026-04-17 acceptance criteria filled in by manager (orchestrator pass)
- 2026-04-17 executor landed:
  - New `benchmarks/fib/fib_vm.rs` — same recursive fib as
    `fib.rs` but without `requires` or `println` (neither is a VM
    op yet). Top-level `fib(25);` leaks the value for the driver
    to print.
  - `benchmarks/run.sh` gains a `Resilient (VM)` row in the fib
    bench comparing to the tree walker and the other-language
    baselines.
  - Ran `./benchmarks/run.sh` on Apple M1 Max. **Result: 12.9×
    speedup.** fib(25): 30.8 ms (VM) vs 396.8 ms (interp).
  - `benchmarks/RESULTS.md` regenerated with fresh numbers and a
    new paragraph under the fib table describing the VM-vs-interp
    result and noting the Value-clone perf cliff as a future
    follow-up.
- 2026-04-17 context: the 3× target from RES-076 is **smashed**.
  The VM also beats Python 3 (35.2 ms), Node.js (64.1 ms), and
  Ruby (70.8 ms) on this benchmark. Lua (8.5 ms) and native Rust
  (1.7 ms) remain ahead.
- 2026-04-17 verification: `cargo test` 206 unit + 1 golden + 11
  smoke = 218 tests default. `cargo clippy -- -D warnings` clean.
  No source changes — this ticket only touched `benchmarks/` and
  the RESULTS doc.
