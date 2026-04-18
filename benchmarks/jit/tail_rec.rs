// RES-168: JIT TCO microbench — accumulator-style tail recursion
// over an input big enough that a non-TCO build would overflow
// the host thread stack.
//
// Without TCO, each `return sum(n - 1, acc + n)` allocates a
// fresh frame on the host stack; at n = 1_000_000 we'd need ~1M
// frames and the process would crash. With TCO, the recursive
// call lowers to a back-edge jump and the whole computation runs
// in a single activation.
//
// Expected result: sum(1_000_000, 0) = 500_000_500_000
// (n * (n + 1) / 2 for n = 1_000_000).
//
// Run with:
//   cargo run --release --features jit -- --jit benchmarks/jit/tail_rec.rs
//
// The driver prints the program's i64 return value; `time` in
// front captures wall-clock to paste into RESULTS.md.

fn sum(int n, int acc) {
    if (n <= 0) {
        return acc;
    }
    return sum(n - 1, acc + n);
}

return sum(1000000, 0);
