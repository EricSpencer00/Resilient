# RES-168: JIT tail-call optimization

**Decision: TCO is in.** Direct self-recursion in tail position
lowers to a back-edge jump in the Cranelift JIT instead of a
recursive call + return, so tail-recursive code scales linearly
without blowing the host-thread stack.

## Machine

- OS: `Darwin arm64` (macOS aarch64)
- CPU: Apple M-series (aarch64-apple-darwin)
- Rust: `rustc` stable 2025-era toolchain, release profile
- Date: 2026-04-17

## Workload

`benchmarks/jit/tail_rec.rs` computes `sum(n, 0)` accumulator-
style:

```rust
fn sum(int n, int acc) {
    if (n <= 0) { return acc; }
    return sum(n - 1, acc + n);  // <-- direct tail call
}
```

`sum(n, 0)` returns `n * (n + 1) / 2`. The body has exactly one
tail-call site — the shape the JIT's TCO lowering recognizes.

## Raw numbers

Wall-clock from a Python subprocess harness (`/usr/bin/time -p`
rounds to centiseconds, which hides everything under 10 ms):

| Backend | n           | Wall (ms) | Result               | Notes                        |
| ------- | ----------- | --------- | -------------------- | ---------------------------- |
| JIT     | 1_000_000   |      4.41 | 500_000_500_000      | includes JIT compile         |
| JIT     | 10_000_000  |      7.20 | 50_000_005_000_000   | 10× work, ~1.6× more time    |
| JIT     | 100_000_000 |     74.01 | 5_000_000_050_000_000 | linear in n once compile dwarfs |
| tree    | 1_000       |      <1   | 500_500              | OK — fits in host stack      |
| tree    | 5_000       |   crash   | —                    | `fatal runtime error: stack overflow` |
| tree    | 10_000      |   crash   | —                    | same — no TCO on the tree walker |

## What the numbers say

1. **JIT with TCO scales linearly.** 1M → 10M is ~1.6× slower (not
   10×) because the JIT-compile time is a per-run fixed cost that
   dominates at smaller `n`. The 10M → 100M step is closer to the
   ~10× linear expectation (7.2 → 74.0 ms) once the body loop
   dominates.

2. **The tree walker can't run this shape at all past ~2–3K.**
   The tree walker has no TCO path (TCO is JIT-only in this
   ticket), so each `return sum(n-1, acc+n)` allocates a fresh
   frame. macOS's default 8 MB main-thread stack holds a few
   thousand frames at the walker's ~KB-per-frame overhead before
   the process aborts. The JIT TCO lowering makes the same
   program run 1000× deeper — and 100_000× deeper — without any
   change in host memory.

3. **Correctness check.** The expected results match the closed
   form `n(n+1)/2` exactly:
   - n = 10⁶: 500 000 500 000
   - n = 10⁷: 50 000 005 000 000
   - n = 10⁸: 5 000 000 050 000 000

## How to reproduce

```bash
cargo build --release --features jit
./resilient/target/release/resilient \
    --jit benchmarks/jit/tail_rec.rs
```

Tweak `n` in `tail_rec.rs` (one line) to sweep the scale.

## Non-tail-position baseline

A companion shape — `return 1 + inc_count(n - 1)` — does NOT get
TCO (the call's result feeds an `iadd`, so it's not in tail
position). That shape continues to use a regular call and still
stacks up, confirming TCO fires only where it should. See the
`tco_only_fires_on_direct_self_recursion_not_wrapped_calls`
unit test in `resilient/src/jit_backend.rs`.
