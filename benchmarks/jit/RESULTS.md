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

---

# RES-175: leaf-fn inliner

**Decision: inliner is in.** Calls to trivial leaf functions
(≤ 8 AST nodes, no calls / loops / match, not self-recursive)
are lowered by splicing the callee body into the caller instead
of emitting an indirect-call shim.

## Workload

`benchmarks/jit/leaf_heavy.rs` calls `plus_one(i)` 10 000 000
times in a tight `while` loop, accumulating the results:

```rust
fn plus_one(int x) { return x + 1; }

fn loop_bump() {
    let acc = 0;
    let i = 0;
    while i < 10000000 {
        acc = acc + plus_one(i);
        i = i + 1;
    }
    return acc;
}
```

`plus_one`'s body is a 5-node tree (`Block → Return → Infix →
Id + IntLit`) — safely under the 8-node cap.

## Raw numbers (Darwin arm64, release, 10 samples, p50)

| Workload            | inliner OFF | inliner ON | speedup |
| ------------------- | ----------- | ---------- | ------- |
| leaf_heavy (10M)    | 16.94 ms    | 10.66 ms   | **1.59×** |
| fib(25)             | ~4.30 ms    | ~4.26 ms   | ~1.00×  |

Min-times track p50 closely (within ~1 ms), so the speedup
isn't noise-amplified.

## What the numbers say

1. **leaf_heavy runs 37% faster with the inliner on**, above
   the ticket's 30% threshold. Each iteration saves: one
   indirect call + one return + a Cranelift local-function
   import — roughly 5 instructions per call × 10M iterations.

2. **fib(25) is unchanged.** fib's body contains `+ fib(n-1) +
   fib(n-2)` — two calls, so `has_disqualifying_construct`
   rejects it as non-leaf. The inliner scans the body on each
   call-site lookup and bails out in O(n) of body size, which
   for fib is tiny.

3. **Self-recursion guard prevents infinite inlining.** A
   unit test (`inliner_rejects_self_recursion`) pins this
   explicitly — `is_trivial_leaf` returns false when the
   callee name matches the enclosing function's name.

## How to reproduce

```bash
cargo build --release --features jit
./resilient/target/release/resilient \
    --jit benchmarks/jit/leaf_heavy.rs
```

Toggle the inliner by flipping the `if let Some(...)` block in
`resilient/src/jit_backend.rs`'s `CallExpression` lowering
arm to `if false && ...`.
